# Operation progress windows

Copy, Move and Delete use `JobProgressWindow`, with the same layout and controls.
Each confirmed operation has one owned, modeless window identified by its exact
job ID and window session. Multiple operations may have visible windows at once.
The Jobs window is a session ledger; Open displays a selected operation's progress
or retained diagnostics. It does not mix an active transfer with previous jobs.

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

The chart shows a separate horizontal overall-completion fill. The speed curve
retains the full history from time zero inside that completed region. Its vertical
scale includes earlier peaks. Unknown totals retain an indeterminate indicator;
no fictional percentage is shown. One file's byte meter describes written bytes,
not verified publication; it may reach 100 while the phase says Verifying. Overall
completion remains below 100 until successful completion is reported by the backend.

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
are retained in Details; Delete shows its current item and remaining count. The
shared view can host per-file meters; current serial execution has one active lane.
Adding concurrent lanes must preserve distinct immutable source/target identities.

## Qualification

The November 11 follow-up adds focused native-window tests for separate operations,
Trash/permanent Delete progress, X/Escape cancellation, command refusal, exact-ID
Pause/Resume, completed-job reopening and retained session history. Meter tests
cover unknown/partial totals, retries, verification and changes invalidating totals.
UI snapshots cover themes, small windows and enlarged fonts. Final integrated
build/runtime qualification is recorded in the settings/UI follow-up report;
source implementation alone is not evidence that the installed executable changed.

Native Delete from expanded panel branches uses this same confirmation and progress
component. Its immutable capture keeps provider identity in the backend; unsupported
archive writes remain disabled. The [12 November integration record](../verification/2026-11-12-tree-delete-integration.md)
covers real native tree deletion, the shared Delete window, full affected suites
and the exact installed debug executable.
