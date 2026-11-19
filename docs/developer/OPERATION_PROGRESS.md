# Operation progress windows

Copy, Move and Delete use `JobProgressWindow`, with the same layout and controls.
Each confirmed operation has one owned, modeless window identified by its exact
job ID and window session. Multiple operations may have visible windows at once.
The Jobs window is a session ledger; Open displays a selected operation's progress
or retained diagnostics. It does not mix an active transfer with previous jobs.

**Status:** producer/UI behavior has bounded headless-fixture qualification.
Concurrent scheduling and the native namespace registry described below are
implemented in the current source; their real-storage qualification and fresh
application delivery remain pending.

## Lifetime and controls

- Cancel in the confirmation form admits no destructive work and no Jobs receipt.
- X, Escape and Cancel operation in active progress first open an in-window
  cancellation confirmation for that exact job and window session. Continue is
  the default; Continue, Escape or closing the confirmation leaves the operation
  running in its existing pause state. Other operation windows remain usable.
- Confirming Cancel operation requests cooperative cancellation. A rejected
  command leaves the window visible with an actionable retry. A completion or
  replaced/stale window session while the question is open cannot send a new
  cancellation. Completed progress closes without asking.
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
  edited item is still revealed under its new name. Notices of jobs that end
  Completed with errors are titled "File operation completed with errors", never
  "failed", and say that changes were not undone only when a target did not complete.
- A completed or completed-with-errors Copy, Move, Delete, Rename, New file, New
  folder or Properties job refreshes every native panel showing an affected folder as
  soon as its terminal progress is applied, without waiting for a watch notification.
  A flat panel re-reads the folder, superseding an in-flight same-folder background
  re-read (a watch refresh, which may predate the change); any other load, a directory
  comparison or a drag from the panel still defers it. A tree panel whose root
  contains an affected folder refreshes its loaded branches. Main-window panel input
  (keys, pointer and wheel) then waits, with a wait cursor, until those re-reads and
  tree refreshes complete, fail or are superseded; a renamed or created entry gets
  the cursor, in a tree once a publication shows it. A Rename, New file or New folder
  also holds input from its confirmation when no other copy or file job is live,
  because it may otherwise run behind that job. Failed or cancelled jobs, a Rename
  name-conflict question and a refresh that cannot start at once never hold input;
  failures are reported as before. The main window releases the hold after 3 seconds
  in any case.
- Closing Jobs only closes the ledger. It does not cancel independent operations.
- Successfully completed automatic progress closes. A manually reopened progress
  window stays open; so does manually opened Jobs.
- Pause requests a safe backend checkpoint. Until acknowledged the UI says
  Pausing; it must not claim a native call has stopped. Resume and Cancel remain
  available while paused. Cancellation wakes paused waits and takes precedence.
- Finished receipts stay for the session, with separate clear-finished and
  clear-finished-without-errors actions. Metadata outcome details follow the same
  lifetime: there is no last-16-jobs eviction. Results are keyed by operation ID
  for constant-time lookup/replacement; each result still has at most 32 sampled
  outcomes plus complete totals. Clearing also removes matching details, and late
  receipts cannot resurrect cleared jobs. Replaced, cleared, late and shutdown
  metadata payloads go through backend reclamation rather than being freed on the
  UI thread. No queue persistence is implied.

## Admission policy and waits

Requests default to `TransferStartPolicy::Immediate`. The accepted scheduler
contract makes these jobs runnable concurrently, subject to configured worker,
physical I/O and safety admission. The dialog's **Wait for other operations to
finish** option starts unchecked for every new draft and is not persisted.
Confirmation freezes the choice; rejection, reconfirmation and Retry retain it.
Other submission routes default to Immediate.

`WaitForEarlier` atomically captures earlier unfinished Copy/Move admissions,
excluding self and future jobs. Paused/decision-waiting predecessors are unfinished.
Escape and X on a decision window defer it like Decide later, so a deferred decision
keeps its job unfinished until Jobs → Resolve answers it: a Copy/Move decision holds
later destructive mutations behind the predecessor fence, and a Rename or metadata
decision holds the single mutation lane while its worker checks for the answer every
25 ms.
Failure or cancellation releases this scheduling wait only after actual native
cleanup/retirement. The separate destructive-operation fence still requires
verified success. Later ready jobs may bypass waiters and unfilled handoffs.

`EarlierTransfers` means an outstanding explicit dependency; `Resources` means
actual I/O, namespace or scope admission is blocked. Selecting a policy alone is
not a current wait. While waiting, a Copy/Move shows Queued, Queued — waiting for
earlier transfers or Waiting for available capacity. It shows Discovering only while
its discovery is actually scanning and never shows Calculating (see ETA under
[Verification and charts](#verification-and-charts); Delete differs there). Clear
wait reasons on acquisition, cancel-request and terminal publication. Cancellation,
pause and decision state take precedence over ordinary wait/discovery presentation.

Each job retains one whole-job namespace share and its complete native scopes
through actual provider destruction, plus one retained slot identity of the
configured global disk-I/O gate. The slot consumes capacity only while the job
executes native steps:

- Pause, user decisions and namespace-grant waits return the slot. The job readmits
  it through the FIFO gate before its next native step (Waiting for available
  capacity); a paused job takes neither lease.
- At each step boundary a running job yields its slot while a call-scoped request
  (checksum, thumbnail, viewer, folder-size or file-list I/O) is queued, so such a
  request waits for at most one step of each transfer, never for a whole job. Jobs
  queued for a slot do not cause yields; they wait for a running job to finish,
  pause or wait. Discovering one selected root's tree for progress totals is a
  single step, so a large or slow tree delays both this yield and an acknowledged
  pause until that discovery returns (cancellation is still honored inside it).
- Waits for decisions, pause, resume, admission and namespace changes, and terminal
  delivery, are woken by the controller's cancel, pause, resume and shutdown
  notifications; no waiting job polls its flags on a timer (a one-second re-check
  only guards a flag changed without a notification).
- Checked teardown reclaims a released slot without queuing, so cancellation never
  waits for capacity (it may briefly exceed the limit). Quarantine returns the slot
  but keeps native authority, the namespace share and the claim.

Initial acquisition takes the namespace share and then the FIFO disk slot. Legacy
namespace consumers take their complete namespace before any disk slot, and no
slot holder waits for a namespace share, so this order cannot deadlock. One job
performs one native call at a time; a disk-I/O limit of one runs one job's steps at
a time. Future within-job multiple streams still require positive local SSD
classification at both endpoints and shared budgets.

Namespace admission grants a claim only when its complete footprint is disjoint
from every granted claim and every earlier waiting claim; an opaque claim excludes
all others. A refused grant waits for a registry change (a grant, completion or
quarantine; any change while admission is momentarily closed) and revalidates at
least once per second, holding no disk slot. A pin of another job that cannot be
re-probed only defers the grant; it is never recorded against either job. All
namespace changes (stage creation, publication, directory creation, removal) of all
jobs are serialized by one registry turn, which spans the native call. A change of
a running job that granted peers refuse (opaque, latched or intersecting) mutated
nothing; the job records those peers and repeats the refused step, without a prompt,
once one of them changes (its turn, a re-probe, removal or quarantine), holding no disk
slot. Its own turns never end that wait. A peer latched opaque clears only by its
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
owners close, so an external change to any pinned binding (for example a renamed
source or destination parent) is still latched and reported as a job error, as for
jobs with peers. See the
[admission and safety contract](../superpowers/specs/2026-11-07-copy-predecessor-fence.md).

## Discovery and truthful progress

Backend `OperationDiscovery` snapshots distinguish NotStarted, Scanning, Complete
and Unavailable. Discovery counts metadata only, including selected roots and
regular-file logical lengths. A partial or unsupported count is never a percentage
denominator. The discovery phase shows the growing item count and known bytes.

Copy and cross-volume Move may use a separate bounded metadata prepass; execution
continues to use the original captured identities and authoritative revalidation.
A traversal failure or changed counts invalidates the total rather than silently
moving the completion target. Discovery is not filesystem authorization and does
not certify a frozen directory tree. Paths used to execute are raw provider paths,
not the UI strings or the discovery result.

Same-volume native Move keeps its fast rename. It reports selected roots and does
not recursively enumerate or read file contents merely to display a percentage.
System Trash also reports selected roots when its platform callbacks do not expose
all descendants. Permanent Delete can report recursive entries. Labels identify
selected-root scope instead of implying that one renamed/trashed directory is one
recursive file. Links are not followed by destructive traversal.

One acknowledged 250 ms heartbeat (at most 4 Hz) coalesces ordinary discovery,
throughput, verification-pass and wait-state updates. A full outbox leaves the
latest change unacknowledged for retry on the next tick, including a blocked wait
clear; discovery changes do not depend on throughput. Initial, decision and
terminal boundary delivery remain explicit, as do the boundary snapshots of each
preparation attempt, the first namespace-grant wait and pause/resume
acknowledgement. Neither the heartbeat nor the mutation queue's pause and
cancellation rescan republishes a terminal state, so each job reports it once.

## Verification and charts

During readback the headline and per-file meter use `current_file_verified /
verification.readback_bytes(current_file_size)`: full SHA-256 uses file length;
sampled mode uses distinct ranges up to 192 KiB. Empty or unknown budgets show
state without a fabricated percentage. Outside readback the per-file byte meter
describes written bytes. Move's SHA explanation applies only when copying is
required; native rename reads no content and shows no hash percentage.

`verification_pass_id` and `verification_throughput` belong to the same current
copy as its size/copied/verified counters. Every new copy attempt or explicit
Retry of the current copy gets a fresh checked nonzero ID and empty rate history.
Checkpoint Retry keeps actual verified offsets and baselines rates there without
replaying reads. Pause/resume retains the ID/history and excludes paused time;
proof/removal-only Retry creates no copy pass. No current copy, native rename,
proof-only work and terminal cleanup publish zero/empty pass fields.

The pass history contains only current-file readback, at most 240 samples, and
captures fast final read intervals between heartbeat ticks. It excludes writes
and Move proof bytes. UI binds the exact pass ID/history during current-file
verification, including its paused view, and restores whole-job history outside
that phase. The separate verification chart has a clear 0..100% byte scale;
reading 100% does not establish digest success, publication or source removal.

Whole-job completion remains independent and below 100 until backend success.
Its speed history retains earlier peaks and starts at job time zero. Unknown
totals remain indeterminate. Cumulative `verified_bytes` includes destination
readback retries; `validation_bytes` contains additional Move proofs. Neither,
nor their combined throughput, is a whole-job logical verification percentage.
Do not use advisory remaining-work estimates as an unconditional denominator.

Logical committed bytes are separate from actual I/O counters. Retries and
verification reads cannot inflate completion. Cross-volume Move keeps files in
Items remaining until their source removal is confirmed, even when destination
publication has already succeeded. ETA is an explicitly approximate
presentation of remaining logical work and backend-measured rates. Copy/Move
without an estimate show their current activity instead (for example Copying...,
Saving... or Verifying current file...). Delete has no activity label: whenever it
has no estimate (for example while Queued, before its discovery completes or a rate
is measured, or after a failed item) it shows Time remaining: Calculating..., and
its headline reads Discovering: N items until its discovery finishes, already
showing 0 items while it is Queued and discovery has not started. While paused,
both show Paused instead of an estimate.
The backend budgets writes and selected-policy readback for each discovered file:
full length for SHA-256, or up to 192 KiB of disjoint samples. Cross-volume Move
also includes source and destination proof reads both before and after source
capture. Current retained copy/proof state reduces this budget; retry I/O does not
become logical completion, and a repeated removal proof becomes outstanding again.
Native rename and Delete use item rates. Metadata, sync and namespace work have no
byte estimate, so the estimate may disappear again while those steps remain. ETA is
not a guarantee against future stalls or conflicts. See the
[work-accounting evidence](../verification/2026-11-12-transfer-eta.md).

Copy and byte-transfer Move display bytes/s (IEC units) with independent
verification activity. Native same-volume Move displays renamed items/s.
Delete displays items/s, as Windows Explorer does. Current source and destination
are retained in Details; Delete shows its current item and remaining count. Each
per-file meter retains the immutable source/target identities of its own lane.

## Related contracts and evidence

See [native transfers](TRANSFERS.md), the
[initial Copy/Move form](../superpowers/specs/2026-11-07-copy-dialog-design.md)
and [retained native tree-Delete evidence](../verification/2026-11-12-tree-delete-integration.md).
Exact current commands, source hashes, selected results and screenshot limits
remain in the scoped implementation/verification packet.
