# Native mailbox shutdown ownership

12 November 2026; baseline `81659a7`. Continue LIFE-01 without changing the broader
product/UI goal. Root owns archive-size lanes, tree worker investigation, shared
startup integration and qualification; archive navigation and report storage are
separate coordinated tasks. CI batching is independent.

## Evidence before changes

A real file-backed archive root queued in an archive-size lane survives a simulated
native-worker unwind. Final controller/lane release closes that file on the caller:
the targeted regression fails against the baseline. Existing normal shutdown drains
on a native worker, but unwind bypasses that drain.

The analogous tree completion probe already passes before any fix. Source confirms
`BranchResult` contains entry metadata and `RootRegistrationLease` retains a provider
whose captured root was already consumed. Do not label this a reproduced native-mount
cleanup failure. Investigate actual queued BranchJob runtime shutdown separately
before adding capacity or changing that path.

## Implementation

Guard each archive-size lane's existing pending state with one startup lease; two
panes require exactly two slots and no new waiter. Keep existing queue limits,
coalescing, cancellation and normal worker drain. Close only cancels/marks state;
it must not take native owners on the dispatcher. Final undrained storage retires
on the runtime-independent cleanup worker. No per-Drop thread or inline fallback.

Retain original tree filesystem/locale services across independently surviving native
model work if its ownership trace requires it. Do not blanket-wrap metadata-only
completed results. Any newly established native ownership boundary needs its own
concrete test and capacity argument.

## Qualification

Keep RED logs, qualify actual native file closure and final-owner thread, worker
panic and normal shutdown, then run archive-size/tree affected tests on Windows and
Linux once coherent. Coordinate one Cargo owner per target. Record evidence and
remaining boundaries, update the current ledger, build/smoke the debug app, commit
and push the qualified batch. Do not claim full LIFE-01 or UI completion.

## Outcome

Implemented the two guarded archive-size lanes. Both tree probes are passing positive
controls; the production submission helper adds no lifetime guard or capacity. Final
Windows/Linux tests, strict lint, debug deployment and native/software startup are
recorded in [qualification](../../verification/2026-11-12-native-mailboxes.md).
The archive integration findings required the existing two-pane registry capacity to
be preserved; its final derived aggregate retention bound uses 135 slots, bringing
the engine ledger to 303 slots and seven waiters. Diagnostic report ownership is
reproduced and designed separately, with production implementation still pending.
