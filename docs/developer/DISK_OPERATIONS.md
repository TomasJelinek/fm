# Disk-I/O concurrency contract

**Status:** implemented in source. Verification and remaining qualification are
[below](#verification-and-remaining-qualification). The
[user guide](../DISK_OPERATIONS.md) describes the implemented control.

## Meaning and storage policy

`UserPreferences.disk_io_concurrency` uses
`DiskIoConcurrency::{Automatic, Manual(u8)}`. Automatic resolves to the existing
limit of **4**. Manual accepts **1..=32**; Core, UI and persistence reject invalid
values before saving, without assertion, wrapping or clamping. Missing older
preference data means Automatic; existing codec version/error policy remains.

The process-wide shared gate limits **participating bulk I/O**, not every
filesystem syscall or process thread. Find/tree discovery and plugin-private I/O
retain separate budgets. The setting does not size Tokio, CPU decoding/hashing
pools, mutation/capture/cleanup admission or a new per-entry task scheduler.

Admission uses semantic `Shared` or `Exclusive` requests. For call-scoped reads,
only positive local-SSD evidence allows Shared; HDD, network and unknown storage
use Exclusive. Transfers (Copy/Move jobs) request a *retained* Shared slot whatever
the storage class: the job keeps one slot identity for its lifetime but holds
capacity only while it executes native steps. It releases the slot while paused,
waiting for a user decision or for another job's namespace; at each step boundary it
yields the slot while any request is queued (a call-scoped one or another transfer);
and it readmits the slot through the FIFO queue before its next step. A queued
request therefore waits for at most one step of each transfer, not for whole
transfers, later requests are not stuck behind it, and transfers sharing the limit
(even a limit of one) interleave step by step. Most steps are bounded (one 256 KiB
chunk, one name change), but discovering one selected root's tree for progress
totals is a single native step: a large or slow tree delays the yield (and an
acknowledged pause) until that discovery returns. Teardown reclaims a released slot at once, so cancellation cleanup never
queues (it may briefly exceed the limit). The limit thus bounds how many transfers
execute native steps at once; it does not enable streams within one transfer or
establish SSD proof at both endpoints. Existing stricter copy, checksum and
thumbnail job limits remain. See the [transfer contract](TRANSFERS.md) and
[operation progress](OPERATION_PROGRESS.md#admission-policy-and-waits).

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

## Verification and remaining qualification

Windows **208 targeted cases passed**. The earlier **10-case work-budget gate**
is retained as a separate receipt and is not added to that total. Linux
**40 focused tests** and **two scoped strict Clippy checks** passed. Windows UI
library/four integration targets compiled with zero diagnostics.

The fresh private Windows application passed engine and native UI startup smokes:
both exited **0**, without software fallback, and both owned process jobs closed
cleanly. This is scoped startup qualification, not complete native workflow or
performance qualification. Scoped Windows strict Clippy passed with zero errors
or warnings. Native Ubuntu GUI launch is unqualified because no owned graphical
session is available. Coverage is tracked against the checks below:

- Shared shrink/grow with held permits, exclusive holders/waiters across resize,
  cancellation/FIFO and actual I/O admission while older holders drain.
- Invalid/missing persisted values, successful OK, failure/Cancel, unrelated
  saves, Reset and late-load merge/stale-ACK handling.
- A new folder-size team using Manual above four while an existing team retains
  its size, including the physical-thread/frontier accounting above.
- Conservative HDD/network/unknown admission and unchanged stricter copy,
  checksum and thumbnail job bounds.

Matching application/native GUI qualification and performance measurements
require their own receipts. This setting does not establish a
recursive-tree queue repair or scalable retained-tree support.
