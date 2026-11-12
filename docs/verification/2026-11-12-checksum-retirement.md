# Checksum actor retirement

Scope: LIFE-01 checksum ownership after baseline `5388a97`. Digest algorithms,
source revalidation, generation checks, manifest authority, process-wide checksum
quotas, and the single native checksum worker are unchanged. This is a checksum
boundary qualification, not closure of every engine service lifetime follow-up.

## Ownership and capacity

Startup contributes 36 retirement slots: 32 admitted checksum jobs, one serialized
captured intake/rejection, two manifest saves, and one shared guarded transfer
service. One fixed waiter belongs to the captured intake. These cleanup slots do
not increase the existing 32-job/64-MiB process-wide admission quotas; retained
published rows and runs continue holding their original quota.

The private `start_captured` caller contract matters: before it acquires a lease,
its raw request is owned by the enclosing guarded EngineRun. The fixed intake
mutex remains held until a rejected capture is actually retired. This is not a
claim that an arbitrary unpolled caller-created future protects its own raw input.
No provider-bearing input enters an independently spawned checksum task without
its guard already installed.

The same job lease follows Start, blocking preparation output, backend Work queue,
worker admission and execution. Completed but unconsumed preparation outputs are
guarded. Work declares the captured source before retained quota, worker permit
and guarded service, and worker-only unpacking binds payload locals after the
lease so error/panic destruction cannot release that slot early. Native read pins
close inside calculation/verification before the outer captured provider owner.

The service anchor is shared by adapter/backend tasks and each Start, Work and Save.
Its last owner schedules actual provider destruction independently of Tokio. Save
admission reserves queue space before adopting the guarded payload, preserving its
two-save bound. The save guard carries its mutation permit until native cleanup;
normal completion releases save admission only after that cleanup. Published
ChecksumRun/row/event values contain data and quota, not native/provider owners.

No cleanup thread is created per request or destructor. Production construction
uses the common EngineLifetime domain. Standalone fixture constructors take explicit
private startup resources; production constructors never create a fallback domain.
The path-included checksum fixtures include the same retirement primitive.

## Regression evidence

Under `_build/qa/checksum-lifetime-20261112/`, `linux-red.log` records all three
initial cases failing at final destruction on the caller/executor (0 passed,
3 failed, 0.15 s): a dequeued capture waiting on a full outbox plus another queued
capture; an unpolled adapter service at runtime shutdown; and backend Work waiting
for the existing process-wide worker permit. The test process returned 101.

The first Windows guarded implementation passed those three cases in
`_build/qa/task-services-20261112/checksum-green.log`. Additional cases cover:

- A completed preparation result whose JoinHandle is dropped without consumption.
- A rejected capture with a gated destructor: its cleanup slot, serialized intake
  and existing checksum quota remain occupied until destruction completes.
- Explicit cancellation of a still-queued blocking preparation, followed by bounded
  runtime shutdown. It never resolves the source and retires it off the caller.

The first version of the last fixture incorrectly assumed bounded runtime shutdown
itself prevents queued blocking work from running. Windows executed that closure,
so its resolve counter was one. `windows-first-full.log` preserves this failure.
The fixture now explicitly aborts the queued handle while the sole blocking worker
is gated. The resolve-zero assertion remains; production ownership code did not
change for this correction.

Tests that observe a capture's Drop entry also wait for actual global checksum
quota return before releasing their shared fixture. This prevents a following
parallel test from observing the still-destructing quota field. Gated tests release
on unwind and use bounded watchdogs; they launch no external/native child process.

Final Windows evidence in
`_build/qa/task-services-20261112/final-windows-tests-2.log`:

- Full engine unit suite: 919 passed, 7 existing ignored, 6.74 s. All six new checksum
  lifetime cases pass within this suite.
- Checksum actor integration: 4 passed, 0.22 s.
- Checksum manifest verification integration: 64 passed, 0.81 s.
- Checksum backend integration: 65 passed, 0.23 s.

The path-included integration totals overlap helper/primitive tests and must not be
added to the unit count as unique coverage. Existing normal checksum, source-proof,
quota, cancellation and manifest behavior remained covered. Independent read-only
ownership review found no additional actionable gap in this changed seam.

Windows strict engine all-target Clippy also passed in 21.35 s, recorded in
`_build/qa/task-services-20261112/clippy-windows.log`.

## Final Linux qualification

The single-build shared gate is recorded under
`_build/qa/checksum-lifetime-20261112/`. `linux-green.sh` used the existing Ubuntu
24.04 toolchain/SDK configuration, offline locked dependencies, two Cargo jobs and
`_build/linux`. Its private native TMPDIR is recorded in
`linux-green-fixture-root.txt`; no existing directory permissions were changed.

Cargo compiled the engine library and selected integrations once in 1 m 30 s.
`linux-artifacts.py` selected their exact reported executables, validated their
location under `_build/linux`, and recorded hashes. Direct execution then passed:

| Library filter | Passed | Time |
| --- | ---: | ---: |
| `checksum` (including all six lifetime regressions) | 58 | 0.30 s |
| `user_command` | 17 | 0.50 s |
| `task_service_tests` | 8 | 0.11 s |
| `engine_lifecycle` | 8 | 0.12 s |
| `preferences::` | 9 | 0.13 s |
| `fonts::` | 4 | 0.08 s |

| Integration executable | Passed | Time |
| --- | ---: | ---: |
| `checksum_actor` | 4 | 0.21 s |
| `checksum_manifest_verification` | 64 | 0.39 s |
| `checksums_backend` | 67 | 0.32 s |
| `find_actor` | 5 | 0.62 s |
| `find_sort_after_stop` | 1 | 0.37 s |

Each result has its own `linux-green-<filter-or-target>.log`; library filter colons
are replaced by hyphens in filenames. These selected runs had no failures or
ignored tests. Integration/helper coverage overlaps and is not a unique-test sum.

Strict engine all-target Clippy passed in 1 m 31 s in
`linux-green-clippy.log`, using `cargo clippy --locked --offline -p midnight-engine
--all-targets -- -D warnings` with the same explicit SDK configuration.
`linux-green-status.txt` records zero and `linux-green-progress.log` records DONE.
No build/test process remained when the Linux target was released.

The qualified library test executable is
`_build/linux/debug/deps/midnight_engine-0ffc535384c44315`, SHA-256
`29719e00843a31e57e39a2bc18204cf9776489245139b6810e92fb5c673d4dcf`.
All six executable hashes are in `linux-green-executables.sha256`.
Scoped formatting and diff-whitespace checks passed. No independent commit or
push was made by this task.
