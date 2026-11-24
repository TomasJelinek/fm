# Operation progress windows

Copy, Move and Delete use `JobProgressWindow`, with the same layout and controls.
Each confirmed operation has one owned, modeless window identified by its exact
job ID and window session. Multiple operations may have visible windows at once.
The Jobs window is a session ledger; Open displays a selected operation's progress
or retained diagnostics. It does not mix an active transfer with previous jobs.

## Lifetime and controls

- Cancel in the confirmation form admits no destructive work and no Jobs record.
- X, Escape and Cancel operation in active progress first open an in-window
  cancellation confirmation for that exact job and window session. Continue is
  the default; Continue, Escape or closing the confirmation leaves the operation
  running in its pause state. Other operation windows remain usable.
- Confirming Cancel operation requests cooperative cancellation. A rejected
  command leaves the window visible with an actionable retry. A completion or
  replaced/stale window session while the question is open cannot send a new
  cancellation. Completed progress closes without asking. After an accepted
  cancellation the window of a Copy, Move or Delete stays open and then shows how
  the job ended.
- A job window has no close button in its header: its native title bar, Escape and
  the footer's Cancel operation or Close close it. The Jobs window is the same.
- Once a job has ended, its window shows the outcome instead of its progress (no
  speed chart, time or Items remaining): the state, then what was copied, moved or
  deleted and what was not, for example `Files copied: 23,956 · Not copied:
  99,875`, `Items deleted: 108 · Not deleted: 24,284`, `Copy cancelled; files
  already copied are kept.` or `Move cancelled; completed moves remain, and files
  already copied to the destination are kept.` (the engine's sentence from the job's
  message, which Copy details holds too; a Move can leave a file copied with its source
  restored), and counts of skipped items, links copied as links and files kept
  in recovery folders. A clean completion shows its summary. Only errors are listed,
  while the job runs and after it, one row each with the reason and the path
  shortened in the middle; the error count opens the first, and a click, Return or
  Space on a row opens its whole path, message and operation in place. Informational
  notes (a link copied as a link because it could not be followed) and the items a
  cancellation left unprocessed are only counted.
- The window has no Details button. The job's complete details (counters, source,
  destination, diagnostics, every retained error and informational note) are the
  text Copy details copies: from the window's footer once the job has ended, and
  from Jobs (Copy details for the selected job) at any time. What the window showed
  only under Details is either shown always now (the current file's bar, with the
  bytes written or read back as its accessible description) or dropped deliberately:
  a Copy or Move's `Items remaining` line (its totals count files of the total), the
  current file's byte line (its percentage and bar stay) and the separate chart of
  the current file's readback (the whole job's chart keeps its Verify curve and the
  file's bar turns to the readback's colour and share).
- While a job's own window is open, the main window adds no notice when the job
  ends, whether it was cancelled, failed or completed: the window shows the outcome.
  A job whose window is not open (closed while it was cancelling, or a Rename,
  creation or Properties job never opened from Jobs) is reported by a notice.
- Cancellation remains cooperative: finish the safe checkpoint and clean up owned
  unpublished temporary files. Earlier committed changes remain. Jobs retains the
  final result, including cleanup errors; closing presentation never means success.
- A Rename, Trash move or permanent Delete whose change completed but whose private
  Linux staging folder could not be removed afterwards counts its target as completed
  (a renamed entry is still revealed) and adds one issue that starts with the completed
  change and names that folder. The folder is said to remain there only while it has
  the identity it was created with; otherwise the issue says it could not be confirmed
  and that another program may have moved, removed or replaced it. The job ends
  Completed with errors; nothing needs to be retried, and the issue never asks a
  name-conflict question or reads as a cancellation. A Properties (metadata) rename in
  the same situation still applies the item's remaining permission and time changes
  but counts the item as failed (its error is never a name conflict either); a single
  edited item is still revealed under its new name. Notices of such jobs (only when
  no window of the job is open) are titled "File operation completed with errors",
  never "failed", and say that changes were not undone only when a target did not
  complete.
- A Delete (Trash or permanent, from a panel or an expanded tree) pauses on each item
  it cannot remove and asks in the shared decision prompt, headed "Delete needs
  attention" (`OperationEvent::DeleteFailure`, answered by `Command::DecideDelete`
  with the token `delete-error:{job}:{version}`): Retry repeats that step, Skip leaves
  that item and goes on with every remaining item and target, Skip all errors skips
  the job's later failures without asking, Cancel job stops and keeps what is left,
  and Decide later keeps the question for Jobs. On Linux a folder whose items cannot
  leave it (write-protected, or owned by another user) is asked about once, for the
  folder; on Windows each entry's own access decides, so each refused entry is asked
  about.
  The last target failing as a whole in a way Retry cannot change (for example an
  item replaced after the confirmation) is only reported, as nothing is left to decide.
  Folders that keep skipped items are not deleted and not reported as more errors;
  the job's message adds one line per such target, "*target* was not deleted because
  it still contains skipped items." A Delete counts its errors in `failed`: each
  skipped or cancelling failure, each target that ended at a failure that was not
  skipped, and each completed target's issue; `completed` counts the targets
  deleted. While a decision waits, Linux puts what is left of a permanently deleted
  item back under its own name, so the cause can be fixed where it is shown, and
  captures it again with the same identity checks before going on. Windows deletes in
  place without staging, so what is left stays under its own name. While a decision
  waits it closes the entry that failed, which Retry opens again by its name (the
  selected item and a folder whose own removal failed keep their checked handles),
  and keeps the folders above it open for the deletion, so they cannot be renamed or
  deleted meanwhile. A file that a program keeps open without delete sharing is the
  typical Windows case. A Trash move is one step on both platforms (a rename on Linux,
  one Shell `RecycleItem` call on Windows), so it is asked about for the selected item;
  on Windows a folder with an open entry inside is asked about as a whole. On Linux,
  write-protected folders of the confirmed tree that the user owns never stop a Delete
  ([security](../../SECURITY.md)). On Windows the read-only attribute of files and
  folders of the confirmed tree never does: NTFS deletes them with POSIX disposition,
  which ignores the attribute without changing it, and filesystems without it (FAT32,
  exFAT, some redirectors) take the classic disposition, which first clears the
  attribute on the entry itself through a second handle to the same object and puts
  it back on an entry that is not deleted after all. A read-only file with other names
  (hard links), which may lie outside the confirmed tree, keeps it there and is asked
  about.
- Copy/Move enumerate a folder with `TransferSource::next_child_or_failure`. A child
  that cannot be opened once its name is read comes back on its own, and the job
  pauses for that child only (stage Reading the source, the child as source and its
  destination path): Retry opens it again with `open_child`, Skip and Skip all leave
  out that child (a Move keeps its folder and every folder above it) and the folder
  goes on with its next child. A failure of the folder itself (its listing breaks
  off, it was replaced, or, on Linux, it lets its names be read but not opened) still
  pauses once as Reading folder contents, where Skip leaves out the rest of that
  folder. Linux and Windows report child failures (on Windows typically a child the
  user may not read, or a file that another program has open for writing, which a Copy
  source does not read); archive sources keep the default, which reports every failure
  as the folder's. A same-volume Linux Move of a folder the user
  owns that lacks owner write gets owner write through its retained descriptor only
  for its renames (a folder's ".." entry changes) and its own mode back afterwards; a
  prepared Move source is rechecked by identity, size and modified time, so a
  permission change made to fix a failure does not stop its Retry.
- A Move takes the entries of its own folders (a selected folder and the folders
  below it, which it removes once they are empty) with
  `TransferSource::next_move_child_or_failure`. A child that cannot be prepared for
  Move once its name is read (a special file, a mount point, an entry that vanished
  or that another user owns in a sticky folder) pauses for that child only, at stage
  Moving the source: Retry prepares it again with `prepare_move_child` (offered when
  the cause can change, not for a special file or a mount point), Skip and Skip all
  leave it where it is, with its folder and every folder above it, and the folder goes
  on with its next child. What every entry of such a folder would fail alike is
  checked once per folder, before its entries (on Linux `check_moved_folder`: the
  folder is still the bound, protected local folder, and it lets entries leave it,
  which takes write and search permission), so a folder another user owns that does
  not let this user write, or a folder without search permission, pauses once as
  Reading folder contents (Skip leaves out the rest of it) instead of once per child.
  The Linux and Windows sources report child failures (on Windows typically a file
  that another program holds, which a Move cannot hold for its removal; a Windows
  child has no folder-level checks to separate); sources that keep the default
  report every failure as the folder's.
  Under Delete's rule a folder of the moved tree that the user owns and that lacks
  owner write passes that check: on Linux it gets owner write through the descriptor
  the Move retains for it, only for each change that needs it (making the private
  stage, taking an entry into it, removing the stage, removing an emptied folder,
  putting an entry back, and on one volume an entry's own rename out of it), and its
  own mode back as soon as that change returns. So a cross-volume Move empties an
  owned write-protected folder without asking, and a folder that stays (it keeps a
  skipped entry) keeps its mode. The folder that contains the selection is never
  changed: when it does not let the selected item leave (write-protected, or owned by
  another user who does not allow it), the Move stops at preparation, before anything
  is copied, with a permission error that names the item: Retry once the folder is
  writable, Skip and Skip all. Windows needs nothing for this rule: a
  folder's read-only attribute does not stop entries from leaving it; a folder whose
  access list denies deleting its entries refuses the item's preparation with the same
  Retry, Skip and Skip all. Known limit: a Windows cross-volume Move refuses a source
  file or folder that itself has the read-only attribute, before copying it, because
  its removal does not override the attribute (unlike Delete's POSIX disposition;
  [transfers](TRANSFERS.md)).
- Copy/Move stages that still end the job, or the rest of a folder, without an
  item-level Skip, and why:
  - Preparation (stage Preparing) of captured (tree) or archive sources: a failure
    blocks the whole job before anything is copied, with Retry and Cancel only (no
    Retry when retrying cannot help). A selected native item that fails alone (its path
    is the error's) also offers Skip and Skip all: `Preparation::skip_root` plans the
    job again without it, it counts as skipped and as one error, and the others are
    copied. This holds for a single selected item too: Skip
    leaves it where it is and, nothing being left to do, the job ends at once
    (`Preparation::is_last_root`), Failed as a job that completed nothing ends, with
    the item's error and "1 entry or subtree skipped; source data retained." A job
    that requires the exact modified time offers no such Skip: its preflight refuses
    folders, links and virtual roots without a downgrade (`MetadataTransferRequest`),
    so leaving one out would turn that refusal into a partial transfer. A preparation
    failure after recorded cleanup residuals still ends the job.
  - A folder that fails itself (above), a Move's own folder included: its listing
    cannot resume where it stopped, so there is no Retry and Skip leaves out the rest
    of that folder.
  - After an uncertain commit or cleanup whose data stays in place, the job records
    the error and its recovery facts at once and then asks whether to go on, with
    Skip, Skip all errors and Cancel only (`failures::Context::Committed`): never
    Retry, which could replay the commit. Skip leaves the entry as it is (it is not
    counted as skipped: its error is) and goes on with the next entry. In a Move it
    keeps the source folders above the entry when the entry leaves something in them
    (its source, a private stage, recovery data), which a later removal of those folders
    would fail on; after a verified removal whose durability alone is unconfirmed (the
    source and its stage are gone, the copy verified) the emptied folders are removed as
    usual. This covers a publication
    that needs recovery or whose durability is unconfirmed (stage Putting the item in
    place; files, and links in a Copy; the source stays, a Move keeps it), a link stage
    that cannot be cleaned up after its publication, a native Move rename that needs
    recovery (stage Moving the source; the entry waits in its private stage or at the
    destination, with its recovery facts), the cleanup of the private stage or the durability of
    a verified source removal (the source is gone and its copy verified; stage
    Cleaning up or Removing the verified source), a moved directory link whose
    original needs recovery, and a cleanup that fails while the job carries out a Skip
    (a link stage, a Move's retained copy stage, or a stage left after a failed
    file's source was put back; a Copy reports a copy stage and goes on).
    These still end the job, because continuing would be unsafe:
    - a capture that does not match: an object other than the selected source was
      taken from its name, so the provider's results can no longer be trusted;
    - Move invariant failures: a provider result that breaks its contract (an
      unexpected committed destination or retained receipt, a missing cleanup
      authority after a source was removed or put back) or the engine's own
      bookkeeping (more than one active verified file, a missing capture or
      finalizer), where the job cannot know what it would change next;
    - a failed restore during Skip: the source could not be put back and waits in
      its private stage, reported with the job's results; the job stops so that
      nothing else changes before the user sees where it is.
    Folder permissions that could not be given to a copied folder are reported and the
    job goes on; they do not end it.
- A completed or completed-with-errors Copy, Move, Delete, Rename, New file, New
  folder or Properties job refreshes every native panel showing an affected folder as
  soon as its terminal progress is applied, without waiting for a watch notification.
  A flat panel re-reads the folder, superseding an in-flight same-folder background
  re-read (a watch refresh, which may predate the change); any other load, a directory
  comparison or a drag from the panel still defers it. A tree panel whose root
  contains an affected folder refreshes its loaded branches.
- When the user waited for the job (it ended within 5 seconds of first reporting
  that it runs, or sooner; `WAITED_JOB` in `presenter/operations.rs`), the input of each panel showing its change then waits
  until those re-reads and tree refreshes complete, fail or are superseded: a Copy's
  destination (never its sources, which a Copy does not change), a Move's sources and
  destination, and the folder of a Delete, Rename, creation or Properties job. A
  renamed or created entry gets the cursor, in a tree once a publication shows it
  (the tree worker sends a refresh pass's reply only after every view of that pass).
  Only that panel waits: its pointer and wheel input over the panel, with a wait
  cursor, and the keys typed while its file list has focus, including the digit-row
  and bracket shortcuts the native window hook takes. Those keys (at most 64,
  `MAX_HELD` in `held_input.rs`) are
  kept and replayed in order, with the modifiers they were typed with, once the panel
  shows the change; they are dropped if the focus moved to another panel, the command
  line or a dialog meanwhile. The other panel, the command line, the toolbars and the
  function bar stay usable; a file operation requested there for the waiting panel
  waits for its re-read like any other background re-read (see below). A Rename, New
  file or New folder also holds its panel from its confirmation when no other copy or
  file job is live, because it may otherwise run behind that job. That hold lasts as
  long as the job runs, with no time limit, and when the job takes longer than half a
  second the panel's status line says what it does (`Renaming “a” to “b”…`,
  `Creating “name”…`, `Creating folder “name”…`); its completion counts as waited for
  however long it ran. Failed or cancelled jobs, a Rename name-conflict question, a
  refresh that cannot start at once and other jobs that ran longer never hold input;
  a failure or cancellation releases a held panel at once and is reported.
  The wait for the re-reads after a completion is limited to 3 seconds
  (`panel-hold.slint`), counted from
  the completion: then the waiting keys are replayed, input is released, and the
  status line says `Updating the file list…` until the change appears. Escape ends
  either wait at once; it is not replayed, and the keys typed before it are. A panel
  released by Escape is not held again by that job's re-read, only by a later job.
- A file operation (Rename, Delete, New file, New folder) or an expander click
  requested while a background re-read (a watch refresh or a job's re-read) keeps a
  panel's rows on screen runs once that re-read ends, on the targets captured when it
  was requested, unless the panel left the folder meanwhile. Opening an entry does not
  wait; it uses the rows on screen.
- Closing Jobs only closes the ledger. It does not cancel independent operations.
- Successfully completed automatic progress closes. A manually reopened progress
  window stays open; so does manually opened Jobs.
- A Move counts a change it made as unconfirmed (`source_removal_durability_unconfirmed`,
  "Source changes the system could not confirm as saved to disk: N") only when a step
  it takes to save that change to disk fails, never merely because the platform has no
  such step. Any such count gives the job a `CopyClosureReceipt`, so it ends "Completed
  with warnings" with an attention mark in Jobs. Linux syncs the folders a change touched.
  Windows flushes the source and destination folders of a Move's native renames, each
  folder once for the renames the job saves together (before it removes a folder and
  when it ends), and confirms a source removal by delete disposition (a cross-volume
  Move's, a folder a merge emptied, a directory link's original) once the job's
  handles on it are closed, its name is gone from its folder and that folder is
  flushed; a filesystem that does not flush folders leaves nothing to report
  ([Move durability on Windows](TRANSFERS.md#move-durability-on-windows)). A Move on
  Windows, on one volume or across volumes, therefore ends Completed when every change is
  confirmed. A Windows
  removal whose name is still there, or whose folder could not be flushed, is counted
  as unconfirmed and noted as accepted by Windows ("Source removals accepted by
  Windows: N"); both are then true.
- Pause requests a safe backend checkpoint. Until acknowledged the UI says
  Pausing; it must not claim a native call has stopped. Resume and Cancel remain
  available while paused. Cancellation wakes paused waits and takes precedence.
- Finished records stay for the session, with separate **Clear all finished** and
  **Clear finished without errors** actions. A refresh changes the rows of the jobs
  whose presentation changed in place (`navigation_models::update`), so a progress
  report does not rebuild the Jobs list's rows; the model is replaced only when jobs
  come, go or change order. The Jobs window's bindings read the rows through the
  model's tracker (`job_presentation::has_finished` and `find`), so a job that
  finished in place enables the Clear actions and the selected job's details follow
  ([record](../verification/2026-11-24-ui-perf-fixes.md#jobs-rows-in-place)). Metadata outcome details follow the same
  lifetime: there is no last-16-jobs eviction. Results are keyed by operation ID
  for constant-time lookup/replacement; each result still has at most 32 sampled
  outcomes plus complete totals. Clearing also removes matching details, and late
  results cannot bring cleared jobs back. Replaced, cleared, late and shutdown
  metadata payloads go through backend reclamation rather than being freed on the
  UI thread. No queue persistence is implied.

## Admission policy and waits

Requests default to `TransferStartPolicy::Immediate`. The scheduler runs these jobs
concurrently, subject to configured worker, physical I/O and safety admission. The
dialog's **Wait for earlier transfers** option (`operation-controls.slint`) starts
unchecked for every new draft and is not persisted.
Confirmation freezes the choice; rejection, reconfirmation and Retry retain it.
Other submission routes default to Immediate.

`WaitForEarlier` atomically captures earlier unfinished Copy/Move admissions,
excluding self and future jobs. Paused/decision-waiting predecessors are unfinished.
Escape and X on a decision window defer it like Decide later, so a deferred decision
keeps its job unfinished until Jobs → Resolve answers it: a Copy/Move decision holds
later destructive mutations behind the predecessor fence, and a Rename or metadata
decision holds the single mutation lane while its worker checks for the answer every
25 ms. A waiting Delete question marks its job for attention too, so Jobs and the
Delete's progress window offer Resolve for it; when several questions wait, the job
most recently chosen with Resolve shows its question.
Failure or cancellation releases this scheduling wait only after actual native
cleanup/retirement. The separate destructive-operation fence still requires
verified success. Later ready jobs may bypass waiters and unfilled handoffs.

`EarlierTransfers` means an outstanding explicit dependency. The other reasons say
why the job cannot proceed right now: `DiskCapacity` (every configured disk slot
is busy for longer than another job's step), `SameFolders` (another job uses the
same files or folders: one item waits, or the whole job when it would read what
that job changes), `UnconfirmedFolders` (this job or the other has an unknown
footprint, so that one runs alone in the opaque lane, whichever folders they use)
and `OtherOperation` (a Delete,
Rename, new folder or saved list holds or queues for the whole namespace).
`Resources` remains a generic capacity wait. Selecting a policy alone is not a
current wait. While waiting, a Copy/Move shows Queued, Queued — waiting for
earlier transfers, Waiting for disk capacity, Waiting for another job in the same
folders to finish, Waiting for another job: jobs whose folders cannot be confirmed
run alone, or Waiting for another file operation. It shows Discovering only while
its discovery is actually scanning, and Time remaining: Calculating... only while its
totals are known and it runs without an estimate yet (see ETA under
[Verification and charts](#verification-and-charts)). Clear wait reasons on
acquisition, cancel-request and terminal publication. Cancellation, pause and
decision state take precedence over ordinary wait/discovery presentation.

Each job retains one whole-job namespace share and its complete native scopes
through actual provider destruction, plus one retained slot identity of the
configured global disk-I/O gate. The slot consumes capacity only while the job
executes native steps:

- Pause, user decisions and namespace waits return the slot. The job readmits it
  through the FIFO gate before its next native step (Waiting for disk capacity once
  that takes longer than 250 ms, `DISK_WAIT_SHOWN` in `copy_operations.rs`); a paused
  job takes neither lease.
- At each step boundary a running job yields its slot while anything else is queued
  for it: a call-scoped request (checksum, thumbnail, viewer, folder-size or
  file-list I/O) or another transfer. Such a request waits for at most one step of
  each transfer, never for a whole job, and transfers sharing the limit (even a
  limit of one) interleave step by step. Discovering one selected root's tree for progress totals is a
  single step, so a large or slow tree delays both this yield and an acknowledged
  pause until that discovery returns (cancellation is still honored inside it).
- Waits for decisions, pause, resume, admission and namespace changes, and terminal
  delivery, are woken by the controller's cancel, pause, resume and shutdown
  notifications; no waiting job polls its flags on a timer (a one-second re-check
  only guards a flag changed without a notification).
- Checked teardown reclaims a released slot without queuing, so cancellation never
  waits for capacity (it may briefly exceed the limit). Quarantine returns the slot
  but keeps native authority, the namespace share and the claim.

Initial acquisition takes the namespace share and then the FIFO disk slot. Whole-namespace
operations (Delete, Rename, new folder, a saved list) take their complete namespace
before any disk slot, and no
slot holder waits for a namespace share, so this order cannot deadlock. The share
semaphore is fair: a copy started while a whole-namespace operation is queued waits
behind it (Waiting for another file operation). One job performs one native call at
a time; a disk-I/O limit of one runs one step at a time, alternating between jobs.
A job never copies in parallel streams (IO-01 in the
[backlog](../IMPLEMENTATION_BACKLOG.md)).

Namespace admission is an atomic check-and-claim, decided at once under the
registry lock: the job probes only its own pins and compares them with the
committed facts of granted jobs, which every change announces before it mutates
anything. It never waits for a peer's turn and its cost does not depend on what a
peer has published. A selected root that a granted job claims or has deferred is
deferred too, not refused: the job starts, and that item is claimed by its first
change that no earlier-granted job refuses (later jobs never hold an earlier one
back, so these waits cannot form a cycle). A job is refused as a whole only when a
footprint is unknown (opaque: its own, a granted job's or an older waiting job's),
when one of its sources overlaps what a granted job
changes (its in-progress output may be visible), or behind an older waiting job that
shares an item; it then waits for a grant, completion or quarantine and revalidates
at least once per second, holding no disk slot. The admission says which: an unknown
footprint waits with `UnconfirmedFolders`, the two overlaps with `SameFolders`
(`copy_namespace::Hold`). What it prepared and keeps meanwhile
must not refuse the granted jobs' changes: on Windows a prepared Copy item holds no
handle of its own before its first use, and transfers hold folders with write sharing,
so the busy job can still publish into a folder the waiting job will read (a prepared
Move item still holds its guard, see
[Windows sharing](TRANSFERS.md#windows-sharing-while-a-job-waits-and-runs)).
Because grants do not re-probe
peers, a turn that did not announce every binding it affected refreshes every pin of
its job when it finishes. All
namespace changes (stage creation, publication, directory creation, removal) of all
jobs are serialized by one registry turn, which spans the native call. A change of
a running job that granted peers refuse (opaque, latched or intersecting) mutated
nothing; the job records those peers and repeats the refused step, without a prompt,
once one of them changes (its turn, a re-probe, removal or quarantine), holding no disk
slot. It waits with `UnconfirmedFolders` when every refusing peer's footprint is
unknown (opaque or latched), otherwise with `SameFolders`. Its own turns never end
that wait. A peer latched opaque clears only by its
removal; while any refusing peer is not latched, the step is also revalidated once per
second for external changes the registry cannot see. This covers stage creation,
publication, directory creation, and a Move's source capture (repeated from the
capture step; the After proof still rehashes both files before removal), its
empty-directory removal and its verified source removal (whose retry rebuilds the
finalizer and rehashes both files first). On both platforms the reservation made when
a change creates its private directory (a Move stage or a recovery directory) also
covers the later renames into and out of that directory and its removal in that
change, so peers can refuse a Move capture, a native Move or a replacement only
before its first namespace change, except at the Linux replacement's witness
reservation described next (see
[Windows namespace pins](TRANSFERS.md#windows-namespace-pins)). A Linux replacement
still reserves the witness of its staged file inside the recovery directory it has
just created; a refusal there removes that still-empty directory without another
reservation, so the refused change leaves nothing behind and is repeated. If that
removal fails, the publication ends as `RecoveryRequired` and reports the directory.
A capture whose empty directory cannot be removed keeps it for a repeated capture,
which announces it again, or for the job's cleanup, which removes it or reports its
path (see [bounded ordinary Move](TRANSFERS.md#bounded-ordinary-move)).
A refusal by a quarantined peer never clears and fails the entry with the
non-retryable restart error, and a quarantined claim makes later jobs fail at once
and ask for an application restart instead of waiting.

Turns of a job without a granted peer refresh only the pins they announced (no other
claim can observe its facts, and every grant re-probes all planned records). Its
untouched pins are revalidated by the next grant and once more before the job's
owners close, so an external change to a pinned binding of an item the job started
(for example a renamed source or destination parent) is still latched and reported as
a job error, as for jobs with peers. That final revalidation
(`Claim::revalidate_before_close`) leaves out the pins of selected items that a job
stopped by Cancel or an error never started
([Windows namespace pins](TRANSFERS.md#windows-namespace-pins)). See the
[admission and safety contract](../superpowers/specs/2026-11-07-copy-predecessor-fence.md).

## Discovery and truthful progress

Backend `OperationDiscovery` snapshots distinguish NotStarted, Scanning, Complete
and Unavailable. Discovery counts metadata only, including selected roots and
regular-file logical lengths. A link that the job keeps as a link (Links: keep links,
or a link that cannot be followed) counts as an item without content bytes, as its copy
writes none and reads none back; a followed link is discovered as what it points to,
with that file's length. An entry of another kind (a device or pipe) is never copied
and adds no bytes either. A link or junction in the tree therefore leaves the byte
total, the byte-based bar and the estimate known. A partial or unsupported count is
never a percentage denominator. The discovery phase shows the growing item count,
and for Copy and Move the known bytes.

Copy and cross-volume Move may use a separate bounded metadata prepass; execution
continues to use the original captured identities and authoritative revalidation.
A part that cannot be counted (a folder that cannot be read, an item that vanished, a
file without a length, a count stopped by a pause) leaves the counts so far as an
approximation (`OperationDiscovery::approximate`) and counting goes on with the other
selected items; work that turns out larger than counted raises the totals to what is
done (`OperationDiscovery::cover`), so a share of them never passes 100% or goes back.
Only a cancelled count or an overflow leaves the totals unavailable. Discovery is not
filesystem authorization and does
not certify a frozen directory tree. Paths used to execute are raw provider paths,
not the UI strings or the discovery result.

A Copy out of an archive counts its members the same way, from the archive's index
(lengths recorded in the archive, never decoded), and shows each member by its path
inside the archive file
([archive members as a source](TRANSFERS.md#archive-members-as-a-source)).

Same-volume native Move keeps its fast rename. It reports selected roots and does
not recursively enumerate or read file contents merely to display a percentage.
System Trash also reports selected roots when its platform callbacks do not expose
all descendants. Permanent Delete can report recursive entries. Labels identify
selected-root scope instead of implying that one renamed/trashed directory is one
recursive file. Links are not followed by destructive traversal.

One acknowledged 125 ms heartbeat (`CADENCE` in `operation_throughput.rs`; at most
8 Hz per job, Copy, Move and Delete alike) coalesces ordinary discovery,
throughput, verification-pass and wait-state updates. A full outbox leaves the
latest change unacknowledged for retry on the next tick, including a blocked wait
clear; discovery changes do not depend on throughput. Initial, decision and
terminal boundary delivery remain explicit, as do the boundary snapshots of each
preparation attempt, the first namespace-grant wait and pause/resume
acknowledgement. Neither the heartbeat nor the mutation queue's pause and
cancellation rescan republishes a terminal state, so each job reports it once.

## Verification and charts

During readback the per-file meter uses `current_file_verified /
verification.readback_bytes(current_file_size)`: full SHA-256 uses file length;
sampled mode uses distinct ranges up to 192 KiB. Empty or unknown budgets show
state without a fabricated percentage. Outside readback the per-file byte meter
describes written bytes. The headline does not follow the current file; it shows
the whole job's completion (below). Under the chart a transfer always shows the
current file on two lines: its folder (`transfer_directory`, the parent of
`current_path`; for an archive member the archive's path and the folders inside it),
and below it its name (`transfer_name`) with its whole percent rounded down (`45%`, or
`Verifying 30%` during readback), then its own bar (accessible label "Current file
progress", the bytes written or read back as its description). `PathText` leaves out
the middle of a line that is too long, keeping the folder's last part
(`transfer_directory_tail`) and the name's last 12 characters (`transfer_name_tail`,
both set by `current_file_tails` from the shown text). Both lines and the bar keep a
fixed place and size for every file, between files and when there is no current file,
so nothing appears, disappears or moves; every file updates them at the window's
refresh cadence (the engine's progress heartbeat, 125 ms), so a small file may show 100% at once and costs the
copy nothing extra. An empty file is full at once; an unknown size, steps other than
writing and readback (such as Move proofs) and ended jobs leave the bar empty; a pause
keeps it. Move's SHA explanation applies only when copying is required; native
rename reads no content and shows no hash percentage.

`verification_pass_id` and `verification_throughput` belong to the same current
copy as its size/copied/verified counters. Every new copy attempt or explicit
Retry of the current copy gets a fresh checked nonzero ID and empty rate history.
Checkpoint Retry keeps actual verified offsets and baselines rates there without
replaying reads. Pause/resume retains the ID/history and excludes paused time;
proof/removal-only Retry creates no copy pass. No current copy, native rename,
proof-only work and terminal cleanup publish zero/empty pass fields.

The pass history contains only current-file readback, at most 240 samples, and
captures fast final read intervals between heartbeat ticks. It excludes writes
and Move proof bytes. The UI uses the pass ID to tell a readback attempt from the
next (`verification_view`, the file's bar in the readback's colour); it no longer
charts the pass history, since the window charts the whole job in every phase.
Reading 100% of a file does not establish digest success, publication or source
removal.

Whole-job completion remains independent and below 100 until backend success: the
headline's `N% complete` divides the bytes of the files already copied plus those
written of the current file by the discovered byte total (items when the job has no
byte total). Approximate totals keep the bar determinate: it holds below 100% when the
work passes them, until the job ends. Only totals that are unavailable leave it
indeterminate, the headline then counting `N items completed`. Cumulative
`verified_bytes` includes destination readback retries; `validation_bytes` contains additional Move proofs.
Neither, nor their combined throughput, is a whole-job logical verification
percentage. Do not use advisory remaining-work estimates as an unconditional
denominator.

Below the headline a Copy or Move shows its totals as they change, once discovery
has them: `Files: 12 of 2,345 · Bytes: 1.5 MiB of 7.0 MiB` (a Move counts a file
once its source is removed), `Files: 12 of ~2,400 · Bytes: 1.5 MiB of ~7.2 MiB` when
the totals are approximate (part of the tree could not be counted, the work grew
beyond the count, or a skipped or failed item leaves part of it undone),
`Files: 10 · Bytes: 1.5 MiB` only when nothing could be counted (a cancelled count), or
`Selected items: 2 of 3` when only the selected items were counted. While discovery
runs there is no totals line; the headline reads `Discovering: 2,345 items (7.0 MiB)`
and counts on. A Delete counts items only: `Discovering: 1,000 items`, then `Items deleted: 400 of 1,000`
(`Selected items moved to the Trash: 1 of 3` when only the selected items were
counted).

The speed chart covers the job from time zero, and older intervals merged into one
keep their peak rates. Its vertical scale holds still while a job copies (`Scale` in
`job_speed_scale.rs`, kept per job and view by the presenter's chart cache): a sample
above the top raises it at once to the next round value at least 1.25 times that
sample; it comes down only after the samples stayed below 40% of it for 10 seconds of
activity, and then by one round step, provided the slower speed still fits below that
step with the same headroom. Round values are 1, 2 and 5 times a power of ten of the
scale's unit (`200 MiB/s`, `1 GiB/s`, `50 items/s`). Time comes from the samples' own
intervals, never from the number of reports, so the progress interval does not change
the scale. A spike therefore raises the scale once, which then steps down at most once
every 10 seconds; a steady or noisy copy keeps one scale, and a speed that alternates
or spikes repeatedly cannot make it oscillate. An older peak above the current scale is
drawn clipped at the top edge. Pauses, waits and stalls are recorded as idle intervals
and neither lower the scale nor count toward its step down.

The chart itself shows no tooltip. Its explanation (what Write, Verify, Deleted or
Renamed measure, the time axis and the scale) belongs to a small "i" button at the
right end of the legend above the chart (`ChartInfoButton`, accessible name "Chart
details", the text also its accessible description): hovering it shows the shared
tooltip; a click, or Enter or Space once Tab has focused it, opens the same text in a
popup below it (Tab alone only focuses it, so Tab keeps moving through the dialog),
right-aligned with the chart and at most as wide as the chart, until the next click
anywhere or focus leaving the button closes it. Opening it moves nothing.

Logical committed bytes are separate from actual I/O counters. Retries and
verification reads cannot inflate completion. Cross-volume Move keeps files in
Items remaining until their source removal is confirmed, even when destination
publication has already succeeded. ETA is an explicitly approximate
presentation of remaining logical work and backend-measured rates. The backend
averages each rate over the job's activity time with an 8-second time constant (an
interval's weight falls to about a third after 8 more seconds of activity) and
publishes the averages once 1 second of activity has been measured; paused, waiting
or deciding intervals without work are left out. Besides bytes it averages completed
items as the window counts them: selected items, or files (a Move's once their source
is removed) and completed folders for Copy and Move, and the items a Delete has
deleted. A Copy/Move estimate weighs the remaining byte budget at the byte rate
against the remaining items at the item rate, the more toward items the smaller the
files that remain (equally at 1 MiB per remaining item, `SMALL_FILE_BYTES` in
`job_progress_meter.rs`); when only one of the two
can be estimated, such as items without a byte budget, it uses that one. An estimate
is shown only while discovery is complete without skipped or failed items and the
job runs, a Copy/Move also without a wait. A paused job keeps its estimate, marked
as in `About 3 min (paused)`, and resumes from the same averages. Without an
estimate, a Copy/Move says Time remaining: Paused while
paused, Discovering items... while its discovery scans, Calculating... while its
totals are known and it runs without a wait, pause request or decision, and
Unavailable otherwise (queued or waiting, pausing, deciding or cancelling, or totals
that are unavailable); an estimate from approximate totals reads `~2 min`; its current activity (for example
Copying..., Saving... or Verifying current file...) labels the speed chart until the
chart has samples. A Delete shows its activity in that line instead (Queued —
waiting to start, Awaiting confirmation, Preparing, Discovering items..., Deleting
items... while its progress is indeterminate, Pausing..., Paused or Resuming...), and
Time remaining: Calculating... while its progress is determinate and no rate has
been measured. Its headline reads Discovering: N items only while discovery scans.
The backend budgets writes and selected-policy readback for each discovered file:
full length for SHA-256, or up to 192 KiB of disjoint samples. Cross-volume Move
also includes source and destination proof reads both before and after source
capture. Current retained copy/proof state reduces this budget; retry I/O does not
become logical completion, and a repeated removal proof becomes outstanding again.
Native rename and Delete use item rates alone. Metadata, sync and namespace work
have no byte estimate, so the estimate may disappear again while those steps remain.
ETA is not a guarantee against future stalls or conflicts. See the
[work-accounting evidence](../verification/2026-11-12-transfer-eta.md).

Copy and byte-transfer Move display bytes/s (IEC units) with independent
verification activity. Beside the headline (`42% complete · write 187.0 MiB/s ·
verify 160.4 MiB/s`) a running Copy or Move shows the smoothed rates its estimate uses
(`smoothed_transfer_per_second` and `smoothed_verification_per_second`, averaged over
about eight seconds of activity), formatted as the app's sizes in the number locale.
The verify rate shows when the job reads its copies back in full (SHA-256; sampled
readback reads a few KiB per file). Both show at once instead of alternating with the
phase, a rate not measured yet or zero for now reads `—` in its place, and the headline
and each rate keep the width of their widest value (`100% complete`, `write 8,888.8
MiB/s`), so the line does not move as the numbers change (`write_speed` and
`verify_speed` of `OperationRow`, from `copy_speeds` in `job_progress_meter.rs`). A
paused, waiting, deciding, cancelling or ended job, and a Move that only renames
entries, shows no rates; its headline says why. Native same-volume Move displays
renamed items/s. Delete displays items/s, as Windows Explorer does. Current source
and destination are retained in Copy details; Delete shows its current item and
remaining count. Each
per-file meter retains the immutable source/target identities of its own lane.

## Related contracts and evidence

See [native transfers](TRANSFERS.md), the
[initial Copy/Move form](../superpowers/specs/2026-11-07-copy-dialog-design.md)
and the [tree Delete record](../verification/2026-11-12-tree-delete-integration.md).
