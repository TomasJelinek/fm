# Watch ownership at task cancellation and native unwind

Base: b9f8ee4. Scope: LIFE-01 watch tasks and retained tree scopes. This note does
not claim closure of the remaining engine ownership inventory.

The old watch tasks retained raw filesystem/native service references. Each pane's
Shared also retained a TreeScope containing MountedTarget and RootRegistrationLease.
Normal native-loop exit drained that scope, but an unpolled task or a native panic
could bypass the drain and leave final resource destruction to a later handle Drop.

The controller now accepts two startup-reserved leases, one for each
Arc<WorkerOwned<Shared>>, and the existing TaskLifetime. The outer future and its
nested blocking closure each retain the service anchor. This adds exactly two
payload slots and no waiter or separate service reservation. Root integrates them
into the engine startup ledger; production watch construction starts no cleanup
thread and performs no fallible reservation after accepting providers.

Normal scope replacement stays on the model worker. Target and tree Runtime keep
native subscriptions local to their native worker, including unwind. The explicit
normal-exit drain remains. Polling, no-follow validation, cancellation, debounce,
32-directory batch bound and callback generation checks are unchanged.

## Regression evidence

The new fixtures create an actual NativeProvider capture, Session and committed
watch scope. Its filesystem destructor records its thread and blocks behind a gate.
The test retains the other pane so an unavailable two-slot domain proves the gated
mount still occupies the exact remaining pane reservation.

Before the fix, the three Linux tests failed at their final Drop-thread assertion:
0 passed, 3 failed, 0 ignored in 0.20 seconds (compile 1m56s). The cases cover a
retained scope before first task poll, a retained scope after native-loop unwind,
and an unpolled task's final filesystem service reference. The unwind case first
verified that its live native subscription was destroyed on its creating thread
and that the panic bypassed the shared-state drain. The panic is deliberate fixture
behavior, not a production failure.

The initial compile attempt encountered concurrently changing desktop constructor
signatures; it is retained separately and is not counted as regression evidence.

- `_build/qa/watch-lifetime-20261112/linux-red.log`: the three observed RED failures.
- `_build/qa/watch-lifetime-20261112/linux-red-integration-compile.log`: initial
  shared-integration mismatch, resolved by the desktop owner's/root's integration.
- `_build/qa/watch-lifetime-20261112/linux-green.sh`: focused final watch command
  using the existing native Linux SDK/environment, isolated TMPDIR and two test
  threads. Final watch result: **39 passed, 0 failed, 0 ignored in 1.94 seconds**
  (compile 1m11s), including all three new regressions. Runner exited zero.
- `_build/qa/watch-lifetime-20261112/linux-green-desktop-compile.log`: a second
  shared-integration compile failure in desktop editor error-message ownership,
  corrected by that owner before the successful watch run.

Independent read-only review found no actionable issue in guard propagation,
service anchor ordering, subscription affinity or the slot-order assertions.
No OS watcher, authority, security or subscription API changed; no new dependency.

## Final affected Linux gate

After the desktop owner's final test-only completion-wait correction, one fresh
artifact build selected the engine library and eight directly affected integration
targets. Tests ran directly from those reported binaries with isolated
`/tmp/fm-watch-final.SkqT3cuH/_build/tmp`, two test threads and bounded execution.
The native SDK/environment were the existing
`_build/qa/interaction/linux-verification-env.sh` and
`_build/qa/elevation-linux-20261111/sdk.toml`; Cargo was locked/offline with two jobs.
No broad Linux library suite or unrelated UI/native timing run was performed.

| Library filter | Passed | Time |
| --- | ---: | ---: |
| desktop | 16 | 0.46s |
| clipboard | 35 | 0.30s |
| watch:: | 39 | 1.92s |
| user_command | 18 | 0.48s |
| engine_lifecycle | 8 | 0.13s |
| task_service_tests | 8 | 0.08s |

| Integration | Passed | Time |
| --- | ---: | ---: |
| checksum_actor | 4 | 0.18s |
| clipboard | 3 | 0.08s |
| desktop | 11 | 0.49s |
| editor | 3 | 0.26s |
| find_actor | 5 | 0.58s |
| user_command_actor | 2 | 0.12s |
| user_command_process | 1 | 0.00s |
| watch_native | 1 | 1.07s |

All listed filters/targets had zero failures and zero ignored cases. The build
finished in 1m33s; strict `cargo clippy -p midnight-engine --all-targets -- -D warnings`
finished in 1m32s. The final driver exited zero and recorded DONE. The existing
TaskLifetime cases independently cover uninvoked/panicking blocking closures and
native work surviving runtime shutdown. The corrected desktop gate test is present
in `linux-final-lib-desktop.log` and passed.

Reproduction and evidence are under `_build/qa/watch-lifetime-20261112/`:
`linux-final.sh`, `linux-final-progress.log`, `linux-final-build.log`,
`linux-final-lib-*.log`, `linux-final-integration-*.log`, `linux-final-clippy.log`,
`linux-final-status.txt` and `linux-final-executables.sha256`. The last file records
all nine selected executables. The final Linux library test executable SHA-256 is
`8be88459fb1b8a22debb76410a75c78d8f468435578caaa9bfc82d875c4db179`.

Owned-source rustfmt and diff whitespace checks passed. Root owns final Windows
qualification, deployment and commit. No commit/push was performed by this task.
