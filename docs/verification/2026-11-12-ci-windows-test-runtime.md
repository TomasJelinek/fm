# Windows CI Test duration and runtime follow-up

## Observed result

This was a read-only investigation of run `43131426269`, exact head
`6f06ec45d2ca27c9d78bca88d6e2d458c87e97e5`. The
[Windows job](https://github.com/TomasJelinek/fm/actions/runs/43131426269/job/130897995231)
completed its **Test step successfully in 50 m 32 s**, from
`2026-11-12T01:05:51Z` through `01:56:23Z` (3,032 seconds).
At the final `01:58:26Z` observation the job was running native viewer-engine
integration, with no failed Windows step. The whole job and pipeline were not yet
terminal; this note does not label them green. There is no confirmed Test-step hang.

The two nearest earlier Windows samples were incomplete:

| Run | Test duration | Last started gate | Progress |
| --- | --- | --- | --- |
| `43117240558` / `a6d90f3` | 35 m 45 s | `panel_search` | 178 of 281 gates |
| `43122031929` / `3522775` | 40 m 25 s | `tabs_bookmarks` | 226 of 281 gates |

Both stopped on the previously diagnosed WinError 32 cleanup failure after their
last test passed. The latter left 55 gates unmeasured. Their shorter durations were
therefore not full-suite baselines. The successful 50 m 32 s step is the first
complete Windows Test sample among these three compared runs.

While the job was reported active, `gh run view --job ... --log` refused incomplete
logs; the REST job-log endpoint returned `BlobNotFound`, and check-run output had
no text or annotations. Those responses did not identify the current gate, a
blocked test, or a runner failure. The API later supplied the successful completion
timestamp. No cancellation, rerun, workflow edit, or runtime probe was performed.
Prior full logs remain under `_build/qa/viewer-settings-20261111/` as
`ci-a6d90f3-windows-job130850286183.log` and
`ci-3522775-windows-job130866843153.log`.

## Backlog candidate: reduce repeated Cargo startup

The workflow Test step invokes `scripts/ci-workspace-tests.py`. Its gate loop starts
Cargo serially for each selected target; the compared logs report 281 gates and
275 integration target names. The check job limits compiler concurrency to two.
The runner prints each command and forwards output with flushing, but has no
per-gate elapsed heartbeat or subprocess deadline. This is an observability limit,
not proof that a particular timeout or concurrency increase is appropriate.

A future measured optimization could compile selected test executables once per
bounded batch and execute Cargo-reported binaries directly. It is **not implemented
or qualified in this batch**. Whole-workspace build-once is not automatically safe:
per-gate retirement currently prevents generated UI test artifacts from filling the
hosted runner disk. Any replacement must preserve a bounded live-artifact budget.

Before changing execution, map and verify:

- The same platform-specific active feature union, target eligibility,
  required features, disabled harnesses, and library/bin/integration selection.
- Package working directories, Cargo runners and runtime library search paths,
  build-script environment, existing private installations, renderer selection,
  and TEMP isolation. Direct execution must reproduce the relevant Cargo context.
- All doctests and both compile-only and test-enabled examples. They require
  explicit coverage; executable enumeration alone does not preserve them.
- Exact Cargo JSON artifact identity, test arguments and ignored-test behavior,
  fail-fast status, failed-artifact retention, and the existing validated bounded
  retirement rules. No glob-based execution or cleanup expansion.

Measure Cargo startup, compilation/linking, binary execution, and cleanup separately
on the same head and runner class. Compare total duration and peak disk usage while
checking identical executed target/test inventories. Add bounded per-gate progress
only if useful for diagnosing long runs. The serial invocation count identifies a
candidate; it does not establish how much of the 50 m 32 s this change would save.
