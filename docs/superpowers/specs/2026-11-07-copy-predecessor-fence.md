# Copy predecessors and mutation safety

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
