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
Failure or cancellation releases this scheduling wait only after actual native
cleanup/retirement. The separate destructive-operation fence still requires
verified success. Later ready jobs may bypass waiters and unfilled handoffs.

`EarlierTransfers` means an outstanding explicit dependency; `Resources` means
actual I/O, namespace or scope admission is blocked. Selecting a policy alone is
not a current wait. Display Queued/waiting or Waiting for available capacity;
Discovering/Calculating requires actual discovery activity. Clear wait reasons on
acquisition, cancel-request and terminal publication. Cancellation, pause and
decision state take precedence over ordinary wait/discovery presentation.

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
finalizer and rehashes both files first). A refusal by a quarantined peer never
clears and fails the entry with the non-retryable restart error, and a quarantined
claim makes later jobs fail at once and ask for an application restart instead of
waiting.

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
acknowledgement.

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
presentation of remaining logical work and backend-measured rates. It remains
Calculating when discovery/rates are unavailable and does not advance while paused.
The backend budgets writes and selected-policy readback for each discovered file:
full length for SHA-256, or up to 192 KiB of disjoint samples. Cross-volume Move
also includes source and destination proof reads both before and after source
capture. Current retained copy/proof state reduces this budget; retry I/O does not
become logical completion, and a repeated removal proof becomes outstanding again.
Native rename and Delete use item rates. Metadata, sync and namespace work have no
byte estimate, so ETA may return to Calculating while those steps remain. ETA is
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
