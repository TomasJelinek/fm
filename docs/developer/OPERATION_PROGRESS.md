# Operation progress windows

Copy, Move and Delete use `JobProgressWindow`, with the same layout and controls.
Each confirmed operation has one owned, modeless window identified by its exact
job ID and window session. Multiple operations may have visible windows at once.
The Jobs window is a session ledger; Open displays a selected operation's progress
or retained diagnostics. It does not mix an active transfer with previous jobs.

## Lifetime and controls

- Cancel in the confirmation form admits no destructive work and no Jobs receipt.
- X, Escape and Cancel operation in progress request cancellation of that exact
  job. A rejected command leaves the window visible with an actionable error.
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
  clear-finished-without-errors actions. No queue persistence is implied.

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
Full SHA-256 Copy subtracts the current file's written and readback bytes from the
remaining budget; retries cannot inflate this progress. Native rename and Delete
use item rates. Byte ETA for cross-volume Move or sampled verification remains
Calculating until those additional proof/sample budgets can be measured reliably.
ETA is not a guarantee against future stalls or conflicts.

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
