# SSD transfer overlap: benchmark gate

Status: QA experiment executed, 2026-11-10; **no-go from this run because confirmation
is incomplete**. Completed large/mixed measurements show substantial gains, but
repeated multi-minute independent small-file audits interrupted qualification.
See the [results and retained anomaly](../../verification/2026-11-10-ssd-copy-probe.md).
No production scheduler is authorized by this document. The latest user instruction
is to benchmark this PC first and **not
implement SSD concurrency unless copying becomes substantially faster**. This
supersedes the earlier request to proceed directly from an IO-01 design to code.

The user still requires that any eventual overlapping transfer use positively
identified local SSD storage at **both** endpoints. HDD, network, unknown, mixed
or failed classification remains serial. The existing
[concurrency design](2026-11-04-copy-concurrency-design.md) supplies the future
safety requirements. The current
[verification and native Move contract](2026-11-10-transfer-verification-and-native-move.md)
supersedes that older document's BLAKE3 and unsupported-Move descriptions.

## Decision to make

Determine whether 2 or 4 independent file streams improve the current native
Windows verified-copy primitive enough to justify implementing a bounded engine
scheduler. The experiment must measure real copying of B while A is independently
read back, rather than extra threads waiting behind the same exclusive permit.

The substantial-gain gate is a repeatable 20% full-mode throughput improvement;
the coordinating review approved the detailed measurement method below. Workload
sizes remain bounded experiment choices, not new application requirements. A negative or ambiguous
result means record the evidence and leave production copying serial. Quick mode
alone, an in-memory hash benchmark, or a favorable isolated trial cannot justify
the feature. No hash algorithm, buffer size, UI setting or native Move policy is
changed by this experiment.

## Current baseline and why a normal thread test is insufficient

- `copy_operations.rs::execute_input` awaits one worker step at a time. Engine
  construction supplies one transfer-worker semaphore permit.
- `copy_worker.rs::Job` owns one current file. `move_worker.rs` permits one
  published Move proof at a time.
- `midnight-platform-common/src/transfer.rs::io_permit` spends all four slots of
  `midnight_work_budget::shared_io()` for each native source read, destination
  write, sync and readback. Four threads using this unchanged helper still
  serialize their native I/O.
- The buffer is 256 KiB. Full mode streams source SHA-256, synchronizes the stage,
  independently reads the whole stage, checks identity/revision/length/digest and
  publishes safely. Sample mode compares first/middle/last 64 KiB, or all bytes
  for files of at most 192 KiB. The policy remains immutable for each run.

The existing [buffer probe](../../verification/2026-11-10-copy-buffer-probe.md)
used a different runtime and warm cache. It cannot answer this question.

## Isolated harness

All executable harness code, lockfiles, generated output, data and logs live below
`_build/qa/ssd-copy-probe/`. The harness is a separate Cargo workspace, using the
repository's current native Windows transfer service, core contracts,
`midnight-work-budget`, and the already pinned `sha2 = 0.10.9`. It introduces no
new third-party dependency and changes no shipped source or feature flag.

Snapshot these exact current common modules into the harness: `transfer.rs`,
`transfer_proof.rs`, `transfer_retry.rs`, and `transfer_edges.rs`. Save SHA-256
hashes of the originals and copies. Change **only** the copied `io_permit`
acquisition cost from `midnight_work_budget::IO_SLOTS` to `1`. Keep the same shared
four-slot `Gate`, cancellation predicate and permit lifetime. Save the exact diff
and refuse to run if the expected occurrence is absent or appears more than once.
This grants qualified experimental streams capacity; it changes no verification,
publication, retry, native file or cleanup behavior.

Use two explicit engines:

| Engine | Purpose |
| --- | --- |
| `production` | Unmodified `midnight_platform_common::transfer::VerifiedFileCopy`, one worker. |
| `probe` | Snapshotted primitive with the one permit-cost change, 1/2/4 workers. |

Before comparing concurrency, establish that `production/1` and `probe/1` produce
identical output and have median end-to-end times within 5%. Investigate a larger
difference before using the results; do not normalize it away. The source manifest
must identify the current dirty-worktree file contents, not merely a Git commit.

Each worker claims one file from a fixed shared input list, prepares the real
native source, creates a real exclusive stage through the held destination
directory, runs the primitive to `ReadyToPublish`, publishes with no replacement,
explicitly cleans the stage, and drops its owned handles. One worker never issues
concurrent operations on the same file. There is no eager file-wide read or
second buffer-size experiment. A shared start barrier excludes thread-creation
noise while including per-file opens, stage creation, every byte and final close.

The timer stops only after all workers return and all stage cleanup/handle release
is complete. Deleting the successful final outputs for the next trial is outside
the copy timer and has its own recorded duration. Source generation, topology
queries and independent audit are also outside the timer.

The comparison intentionally isolates independent files with unique destination
names. It is a **component qualification**, not a claim that the application
already supports parallel traversal, overwrite decisions, Move finalization or
shared foreground storage arbitration.

## Local topology and fixture limits

Before execution, record the actual source/destination volume and physical disk
mapping, filesystem, native storage classification, CPU, installed RAM, free space,
power mode and whether the machine is on AC power. Both endpoints must be positively
local SSD; two partitions of one SSD are one physical device, not two independent
devices. Never infer this from drive letters, a product name alone or measured speed.
If evidence is unavailable, report that the qualifying experiment cannot run.

Use only newly created owned fixture directories below the QA root. Proposed
screening workloads, all with deterministic nonzero incompressible fixture bytes:

| Workload | Files | Total source data |
| --- | --- | --- |
| Large batch | 32 files of 64 MiB | 2 GiB |
| Mixed batch | 8 files of 64 MiB, 512 of 1 MiB, 4,096 of 4 KiB | 1,040 MiB |
| Small-file guard | 8,192 files of 4 KiB | 32 MiB |

Keep one workload's source set and at most one trial's outputs on disk, below
approximately 6 GiB live fixture data. Retire that owned workload before creating
the next one. Require free space
greater than twice the largest source set plus a 10 GiB safety margin before
creating fixtures and before each trial. The margin is a QA bound, not a new app
policy. Record actual allocated/logical source and output sizes, directory flags
and compression state. Abort the experiment on space pressure or any I/O error.

If screening suggests the threshold is attainable, run a second large-file shape
with eight 256 MiB files, subject to the same free-space and live-footprint checks.
This stays within the current experiment's disk cap; it is not a
claim of cold-media performance if the data fits RAM. Do not purge global caches,
disable security software, change system power settings or run privileged disk
configuration commands. Record cache conditions and concurrent background work.

Cleanup checks each resolved absolute target is within the explicitly owned QA
fixture root and is an owned directory with no substituted/reparse ancestor.
Remove only manifest-listed fixtures. An identity/path mismatch or failed removal
retains the data and reports the exact location; it never broadens a recursive delete.

## Correctness and actual overlap

Use light wrappers around the real source/stage traits to observe intervals inside
the already acquired I/O permit: source reads, target writes, sync and independent
target readback. Track worker/file identity, maximum simultaneous native calls and
time when one file writes while a different file reads back. The harness retains
only the at-most-four active call intervals and aggregate counters, so telemetry
memory is constant. These are overlapping synchronous native method intervals,
not measurements of kernel/device queue depth.

First run a deterministic barrier test: hold A inside target readback and prove B
enters a real target write before A is released with 2 workers. With the unmodified
exclusive permit, the same test must show no overlap. Separately require nonzero
write/readback overlap in the measured native runs. A higher CPU utilization or a
larger thread count is not this evidence.

Every measured full-mode run must pass its normal digest/length, source revision,
sync and publication checks. Independently audit all completed outputs outside
the timer against a fixture manifest. Audit full contents even in sample mode;
this validates the experiment's data, not an upgraded production sample guarantee.
Require exact output file count, unchanged source content and no unpublished
stage artifacts. A failed trial is a correctness failure, not a timing outlier.

Before timing, smoke-test cancellation with a real native write or readback held,
then release it and require cleanup before return. Previously published files and
source data remain; cleanup refusal reports its path. These checks protect the
QA modification from silently bypassing the production cancellation contract.

## Trials, metrics and proposed gate

Run one untimed warmup per engine/policy/workload. Screen all three workloads at
1/2/4 workers for three seeded, interleaved rounds, rotating order rather than
running all serial trials first. Do not run another build or benchmark concurrently.
If neither 2 nor 4 workers approaches a 20% improvement in full mode, stop after
screening and record no-go. Otherwise run seven measured interleaved rounds for
the candidate settings, baseline and sustained workload.

For each trial retain elapsed wall time, bytes and files, bytes/s and files/s,
process CPU time, peak working set, worker count, mode, topology evidence,
verification bytes, cleanup duration and native overlap evidence. Save every raw
trial; use medians plus per-round paired ratios and min/max, not the best run.

Proposed **go** requires all of the following:

1. The one-worker control passes, correctness/audits pass, and actual distinct-file
   write/readback overlap is observed.
2. For full SHA-256, the same candidate worker count has `T1 / Tn >= 1.20` median
   speedup on both large and mixed batches, and `T1 / Tn >= 1.10` in at least five
   of seven paired rounds for each. A 1.20 speedup means 20% more throughput and
   approximately 16.7% less elapsed time; report both figures.
3. The eight-file full-mode confirmation retains median speedup of at least 1.20.
4. The small-file guard has no median slowdown above 5%; buffer/handle counts
   remain bounded and no cancellation/correctness failure occurs.

Quick mode is a separately reported secondary result and cannot rescue a failed
full-mode gate. Ambiguous/noisy results are no-go, not permission to select a
favorable subset. A go result supports preparing the production implementation;
it does not itself qualify or deploy that implementation.

## Work explicitly deferred until a go result

The proposed two-worker, bounded-window engine and ordered namespace commit lane
are builder choices, not newly user-approved architecture. If the gate passes,
an implementation plan must cover **all** of the following before feature completion:

- Retained physical tokens shared by transfer, native provider/host, SSD folder
  scans, thumbnails, image reads, checksums and foreground navigation. A temporary
  exclusive compatibility bridge cannot be the finished feature or regress
  accepted parallel SSD scans/thumbnail behavior.
- Real overlap among files inside one recursive selected root; bounded planning,
  per-directory outstanding children, per-root completion receipts and per-file
  Move source removal. Native same-volume rename remains checksum-free.
- Exact target/source dependencies, ordered commits among ready independent files,
  and versioned per-file decisions. An unrelated conflict cannot globally pause
  independent files; a commit lane must not turn admission order into such a pause.
- Joining every in-flight call on cancellation, owned partial cleanup, bounded
  actionable diagnostics, retained providers and accurate aggregate progress.
- Shared-device fairness, aliases, opposite-direction deadlock prevention, current
  verification modes and 1/2/4-stream end-to-end application measurements.

Foreground usability must measure **actual directory results**, not only rendering.
Today `copy_operations::run_job` holds the shared mutation semaphore for the whole
job, but `tree_actor::WorkerBudget::acquire` takes that semaphore only for maintenance.
Explicit foreground branch work is preferred by `tree_refresh` while maintenance
waits. Watch/`RefreshLoaded` reconciliation deliberately retains the permit through
apply, to avoid publishing temporary Move/stage namespaces. An eventual scheduler
must test real foreground navigation during blocked copy/readback and distinguish
that from deferred maintenance. Simply releasing the mutation gate would weaken
the current ownership boundary and is not part of this experiment.

## Evidence and outcome

The methodology, results and literal no-go decision are recorded in
`docs/verification/2026-11-10-ssd-copy-probe.md`, with links to the raw QA manifest,
source diff, topology and trial data. Six controls, 27 screening and 42 confirmation
runs have complete audits; one further independent audit was deliberately stopped
after repeated abnormal delays. The predeclared small-file and eight-file
confirmation gates remain incomplete. No performance samples are discarded to
force a positive decision. IO-01 remains unimplemented; current copying is serial.
If the result is no-go, keep current production code unchanged and explain that
the measured benefit did not justify the requested concurrency implementation.

The coordinating hardware query found one SK hynix PC801 1 TB NVMe SSD (Disk 0).
C: and D: are partitions of that disk; the experiment uses owned D: to D: data.
The PC has an i7-13800H (14 cores/20 threads), approximately 31.6 GiB RAM and runs
Windows 11 Enterprise with the Balanced power plan. Exact observations and free
space are recorded in `_build/qa/ssd-copy-probe/hardware-observed.json`. The result
cannot establish performance across two independent physical SSDs.
