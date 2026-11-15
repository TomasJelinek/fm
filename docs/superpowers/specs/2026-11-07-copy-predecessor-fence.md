# Copy predecessors and mutation safety

## 13 November policy amendment

**Accepted requirements; concurrent implementation/qualification pending.**
The required Copy/Move policy defaults to `TransferStartPolicy::Immediate`: runnable
subject to worker, I/O and safety admission. An unchecked, non-persisted
**Wait for other operations to finish** option selects `WaitForEarlier`.
Atomically snapshot earlier unfinished Copy/Move receipts and register the new
receipt in one critical section, excluding self and future admissions. Retry
reuses that snapshot; numeric job IDs do not establish admission order. Later
runnable jobs must bypass waiters and unfilled native/drop placeholders.

Keep `Predecessors::wait` success-required for destructive mutations. A separate
finished latch settles the checkbox after every captured predecessor retires,
including failure/cancellation; those same unsuccessful outcomes still refuse
the destructive mutation. A cancellation request alone is not completion:
native work, cleanup and retained owners must actually retire before a receipt
resolves or its admission capacity returns. Paused/decision-waiting jobs remain
unfinished. Cancelling a waiter must not start its I/O or require unrelated work
to finish. Delayed handoffs, abandoned senders and shutdown preserve ownership.

The frozen first cut owns one actual configured global Shared I/O permit, one
whole-job namespace share and the complete atomic native scope set until actual
retirement. Transfers take namespace weight 1; legacy exclusive users take the
named full weight 32. Pause, decisions and pinned scope waits keep the real I/O
slot; no second budget or per-step namespace lease is introduced.

Copy sources claim Read; Move sources and destinations claim Write. Entry scopes
exclude siblings; Subtree scopes cover descendants. Native object, parent/lookup
domain/key and ancestor facts determine intersection, not display paths or generic
case folding. Read/read coexists; intersecting writes wait. Shared parents alone
do not conflict. Unknown authority takes a conservative global-transfer scope
without a shared-to-exclusive upgrade; revalidation errors remain failures.

The Copy-specific cleanup seal retains guards through checked cleanup and actual
native/provider destruction, then releases scopes/namespace/I/O and settles the
finished latch. Ordinary reported cleanup failure can settle failed after that
confirmed teardown. An unconfirmed unwind is Quarantined: retain claims,
configured I/O and admission ownership, and leave the finished latch pending.
The existing generic retirement worker is reused without a generic API rewrite.
Complete scheduling, native geometry and this strong retirement implementation
remain Backend-private. The 53-path policy/producer/UI source batch is not their
acceptance. Headless producer/UI checks do not establish concurrent/native
teardown. The checkbox never weakens the verified-success fence.

## Original 7 November contract and verification - retained history

The copy and mutation actors share an execution gate, but their independent queues
let a later mutation overtake an earlier queued copy. Existing-entry mutations
(rename, delete and metadata edits) will capture the outstanding copy receipts at
confirmation. They asynchronously wait for those receipts before requesting the
mutation gate. Every predecessor must complete verified publication without errors,
skips or cancellation; otherwise the mutation fails with an inspect/reconfirm
message. Creates remain independent, and later copies are not predecessors.

Copy reservations establish FIFO order at backend admission, including native drops
whose UI handoff has not arrived. Each reservation owns its capacity, completion
receipt and a one-shot payload sender. The copy actor waits for the first reserved
payload without holding execution or I/O capacity. Dropping an unclaimed reservation
releases capacity and resolves failure; abandoned slots are pruned on admission.

Only 32 copy reservations can be active. Up to 32 mutation jobs retain bounded
snapshots of those shared outcomes (at most 32 x 32 references); there is no unbounded
completed-job history. Completion is resolved before releasing the copy execution
gate. Cancellation, failed enqueue, panic and abandoned ownership resolve failure.
One shared snapshot is retained per mutation identity; duplicate confirmations do
not allocate another. Invalid confirmations and interrupted mailbox delivery
release the snapshot so the request can be reviewed and retried. Receipts resolve
before their admission permits return, including native reservations that never
receive a UI claim. Actor shutdown closes every reserved payload slot.

Verification: reproduce A running, B queued, Delete(B source); confirm failure and
cancellation preserve the source; exclude later copies from the captured fence;
cancel/shut down waiters; preserve FIFO across delayed native handoff and release
capacity when a reservation is abandoned. Exercise real native copies/deletion in
owned fixtures and use controlled source-read delays only for scheduling.

Verification on Windows: the original A/B/delete regression failed before the
coordinator because B's source had already been deleted. All ten queue tests now
pass, covering verified success, failed/cancelled/skipped/rejected/abandoned work,
native-source panic, dropped consumer, delayed native handoff, future-copy
exclusion, 256 abandoned reservations, and cancellation/shutdown without holding
execution capacity. The mailbox-cancellation retry regression also failed before
the delivery guard and now passes. The engine library suite passes 188 tests with
one ignored benchmark; all engine integration tests and strict all-target Clippy
passed. No core protocol, platform implementation or dependencies were changed by
this correction.
