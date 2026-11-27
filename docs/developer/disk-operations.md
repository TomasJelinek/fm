# Disk-I/O concurrency contract

**Status:** implemented. Tests and what remains unqualified are
[below](#tests-and-qualification). The [user guide](../disk-operations.md) describes
the control.

## Meaning and storage policy

`UserPreferences.disk_io_concurrency` uses
`DiskIoConcurrency::{Automatic, Manual(u8)}`. Automatic resolves to a limit of **4**
(`midnight_work_budget::DEFAULT_IO_LIMIT`). Manual accepts **1..=32**
(`MAX_IO_LIMIT`); Core, UI and persistence reject invalid
values before saving, without assertion, wrapping or clamping. Missing older
preference data means Automatic; existing codec version/error policy remains.

The process-wide shared gate limits **participating bulk I/O**, not every
filesystem syscall or process thread. Find/tree discovery and plugin-private I/O
retain separate budgets. The setting does not size Tokio, CPU decoding/hashing
pools, mutation/capture/cleanup admission or a new per-entry task scheduler.

Admission uses semantic `Shared` or `Exclusive` requests. For call-scoped reads,
only positive local-SSD evidence allows Shared; HDD, network and unknown storage
and local disks of unknown media type (`StorageKind::Local`) use Exclusive. Transfers (Copy/Move jobs) request a *retained* Shared slot whatever
the storage class: the job keeps one slot identity for its lifetime but holds
capacity only while it executes native steps. It releases the slot while paused,
waiting for a user decision or for another job's namespace. The copy actor runs a job's
steps in blocking calls of at most 5 ms (`STEP_BUDGET` in `copy_worker.rs`; a small
file's 6–8 steps take one call instead of one each, WIN-8); a call also ends at each
completed root, entry, question, namespace refusal, Cancel and pause, after the step in
flight. Between calls the job yields the slot while any request is queued (a call-scoped
one or another transfer), and it readmits the slot through the FIFO queue before its
next call. A queued request therefore waits for at most one call of each transfer, not
for whole transfers, later requests are not stuck behind it, and transfers sharing the
limit (even a limit of one) interleave call by call. Most steps are bounded (one 256 KiB
chunk, one name change), but discovering one selected root's tree for progress
totals is a single native step: a large or slow tree delays the yield (and an
acknowledged pause) until that discovery returns. Teardown reclaims a released slot at once, so cancellation cleanup never
queues (it may briefly exceed the limit). The limit thus bounds how many transfers
execute native steps at once; it does not enable streams within one transfer or
establish SSD proof at both endpoints. Existing stricter copy, checksum and
thumbnail job limits remain. See the [transfer contract](transfers.md) and
[operation progress](operation-progress.md#admission-policy-and-waits).

## Live resizing

The shared gate provides limit updates and effective-limit observation.
It tracks active shared holders and an exclusive holder independently of its
configured limit. Shrinking preserves issued permits and stops new admission
until the count fits. Growing never overlaps an exclusive holder. Existing
holders can temporarily exceed a reduced limit while draining.

Preserve FIFO, exclusive-waiter fairness and cancellation. Queued requests retain
their Shared/Exclusive meaning across resizing. Permit Drop (or an explicit
release, which forgotten `Arc` clones observe) returns issued semantic ownership,
not a numeric cost captured under an older capacity. Computing Exclusive as the
current full-capacity cost still races resizing and does not meet this contract.
Blocking waiters evaluate their cancellation probe without the gate lock and
withdraw their queued request on every exit, including an unwinding probe. Async
waiters (retained transfer slots) are woken when they reach the queue head; dropping
the wait withdraws the request.

## Settings transaction

On successful load/save, Backend applies the effective limit before the matching
result/ACK. OK persists through the existing serial settings worker. Draft edits,
previews, Reset drafts, Cancel and failed writes do not change live admission.
UI includes the field in baseline restoration, persistable-value filtering, Reset
and late-load EditedFields merging. A stale ACK cannot roll back runtime policy.

## Folder-size teams and memory

New teams sample the effective limit and choose
`min(limit, available_roots, 32)` root lanes. Existing teams retain their size
until they finish; all subsequent participating I/O uses the live gate.
A reduction affects new admission immediately, while an increase grows the next
team rather than injecting threads into a running scoped calculation.

Existing team admission stays at two ordinary scans plus at most one tree scan
per pane. Four simultaneous teams at Manual32 can therefore reach **128 root
worker threads**, while the shared gate admits at most 32 participating I/O
holders after any preexisting holders drain. This is not 32 total process threads.
Each root has a **16 MiB frontier ceiling**, allocated lazily; this is neither
preallocation nor a total process RSS bound. Higher concurrency can use more
memory. Automatic preserves the existing four-lane behavior.

## Tests and qualification

Tests cover:

- the gate (`midnight-work-budget/src/tests.rs`): shared shrink/grow with held
  permits, exclusive holders and waiters across resizing, cancellation, FIFO order and
  admission while older holders drain;
- the setting (`midnight-platform-common/tests/disk_concurrency_preferences.rs`, the
  engine's and UI's preference tests, the acceptance Options scenarios): invalid and
  missing persisted values, OK, failure and Cancel, unrelated saves, Reset and
  late-load merge with stale acknowledgements;
- folder-size teams (`midnight-engine/src/folder_sizes/batched/fixtures.rs`): a new
  team using Manual above four while an existing team keeps its size;
- retained transfer slots (`midnight-engine/src/copy_io_slot_tests.rs`): a paused,
  waiting or running transfer lets queued exclusive and later shared requests run;
- conservative HDD, network and unknown admission, with the stricter copy, checksum
  and thumbnail job bounds unchanged.

Performance at higher limits has not been measured. The setting does not change
recursive-tree queueing or the memory a retained tree uses.
