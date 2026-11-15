# Operation progress windows

Copy, Move and Delete use `JobProgressWindow`, with the same layout and controls.
Each confirmed operation has one owned, modeless window identified by its exact
job ID and window session. Multiple operations may have visible windows at once.
The Jobs window is a session ledger; Open displays a selected operation's progress
or retained diagnostics. It does not mix an active transfer with previous jobs.

**Status:** producer/UI behavior has bounded headless-fixture qualification.
Concurrent scheduling, native namespace/retirement cutover and fresh application
delivery remain pending; the scheduler section below is the accepted contract.

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

The frozen first cut retains one real configured global I/O slot, one whole-job
namespace share and complete native scopes through actual provider destruction.
Pause, decisions and already-pinned scope waits intentionally keep the I/O slot.
Initial I/O/namespace acquisition is a try-pair with rollback; legacy consumers
remain exclusive. One job performs one native call at a time; limit one serializes
jobs. Future within-job multiple streams still require positive local SSD
classification at both endpoints and shared budgets. See the
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
throughput, verification-pass and wait-state updates. There is one publisher.
A full outbox leaves the latest change unacknowledged for retry on the next tick,
including a blocked wait clear; discovery changes do not depend on throughput.
Initial, decision and terminal boundary delivery remain explicit.

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
