# Parallel targeted tests — 12 November 2026

The reusable `scripts/run-targeted-tests.py` runs explicit Cargo-reported headless
libtest artifacts after one coordinated compilation. Its defaults bound execution
to four processes and divide available CPUs between their libtest threads. Multiple
filters of one executable share a listing and run; overlapping tests execute once.
Each requested filter must match, and executed/ignored counts must cover the listed
selection. Per-artifact temporary/render output and logs are separate. Tests retain
package working directories and caller-supplied provider environment.

The runner preserves successful/failed artifacts, has bounded process deadlines,
stops admitting processes after failure, and drains or terminates only owned child
handles. It does not replace the CI workspace executor or parallelize interactive
native input and renderer measurements. See the [usage contract](../developer/TEST_EXECUTION.md).

## Evidence

- The focused Python suite passed **14 tests on Windows in 5.512 s** and **14 on
  Linux in 3.425 s**. Actual subprocesses rendezvous over a local socket to prove
  overlap and bounded admission. Regression cases include exact selection,
  coalescing, no matches, anchored counts, private output, failed launch, timeout,
  interruption, exceptional wait and retrying retained-child cleanup. An unrelated
  child remains alive during owned-process cleanup tests.
- The reusable runner passed **39 actual Rust tests** from the final Cargo
  receipts: core selected-navigation 2, two UI filters coalesced into 27 tests,
  and final HTML host-input tests 10. There were three concurrent processes,
  six libtest threads per process within the 20-thread budget, and no cleanup errors.
- The earlier coordinated Windows artifact gate passed **19 focused runs / 229
  test invocations in 22.587 s**, using four processes and five threads each.
  One routing test intentionally appeared in two filters of that temporary harness;
  the reusable runner removes such duplicate executions. This was a warm targeted
  run, not a comparison against a matched serial baseline or a full CI speedup.

Logs and receipts are under `_build/qa/viewer-physical-keys-20261112/`:
`parallel-runner-tests-windows.log`, `parallel-runner-tests-linux.log`,
`targeted-runner-qualified/summary.json`, `parallel-test-results.json`,
`metadata-windows.json` and `app-build-windows.jsonl`.

The first temporary run exposed an obsolete test expectation that panel Alt+Up/Down
were unbound; it was corrected for the requested selected-item navigation while
retaining Command history and Search/Other exclusions. A temporary unanchored
zero-test classifier also misread `120 passed`; the reusable runner parses complete
anchored summaries and has a dedicated regression. An exploratory nonexistent
filter was rejected as intended, and its failed result was retained.

An independent read-only review found a retained-child cleanup edge case; the
implementation and real-process regression were corrected before final qualification.
No throughput claim is derived from mocked or rendezvous-only test fixtures.

## Subsequent targeted recheck

The 14 runner regressions passed again on Windows in **5.380 s** (5.866 s
including interpreter startup), with temporary files under `_build/tmp`. Evidence:
`_build/qa/viewer-physical-keys-20261112/parallel-runner-recheck.log`.
The tests demonstrate actual overlap and cleanup, not a universal speedup factor.
The machine exposes 20 logical CPUs; the default budget distributes those between
at most four admitted target processes. Independent current feature batches use
this runner after coordinated compilation; foreground scrolling measurements keep
an exclusive quiet interval.

CI run `43200260689` exposed an unrelated archive lifetime test race on Linux:
`published_sort_lookup_does_not_retain_retired_map_or_mount` checked its weak
source immediately after acknowledging snapshot destruction. The snapshot and
guarded archive mount retire on different workers; the first acknowledgement does
not imply the second destructor has run. The fixture now receives a notification
from actual source destruction before checking no ownership remains, with a
bounded timeout and an assertion that destruction occurred off the caller thread.
This preserves the lifetime check without sleeps or serializing the test suite.
The focused Linux engine run passed all four `archives::size_sort_tests::` tests
as part of the 27-test tree command batch: four concurrent processes, total
libtest thread budget four, no cleanup errors. Evidence:
`_build/qa/tree-user-commands-20261112/linux-run-1/summary.json`.
This verifies the local test fix; it does not make the earlier CI run green.

## Independent plugin workspaces

`--workspace` explicitly selects a nested Cargo workspace. The repository retains
ownership of the common `_build` output boundary, and selected package manifests
must belong to that workspace. Rust runtime lookup respects its toolchain.
Multiple simultaneous runner invocations need explicitly divided thread budgets;
there is no cross-invocation global scheduler.

The five added regression cases failed against the old API, then the focused
Windows suite completed **19 tests in 5.760 s: 18 passed, one skipped**. Windows
could not create the directory symlink for the last case without additional
privileges. Ordinary nested-workspace execution, metadata mismatch, sibling
manifest rejection and lexical/escaped paths were verified; the real symlink case
also runs on Linux. Existing real-child concurrency and cleanup cases remained
green. Logs: `_build/qa/parallel-tests-20261112/workspaces-red.log` and
`workspaces-green.log`.

The same final runner suite passed **all 19 tests on Linux in 3.551 s**, with no
skips, including creation and rejection of a real directory symlink. Evidence:
`_build/qa/retained-seek-20261112/runner-nested-linux.log`. Independent review
confirmed metadata/workspace matching, manifest confinement, common build/log
ownership and workspace-local Rust runtime lookup.

## Follow-up CI shortcut regression

Run `43206519259` on commit `c16e99a` passes the earlier archive-retirement
regression on Linux, then fails `salamander_shortcuts` because its unsupported
modifier matrix still includes Alt+Down. That chord is now the requested
next-selected-item action. The test explicitly verifies Alt+Up/Down and retains
negative Alt+Shift+Up/Down coverage; no production mapping is changed.
The corrected Windows shortcut target and selected-navigation target each pass,
alongside 77 viewer regressions in three concurrent headless processes with a
six-thread budget. Evidence is in
`_build/qa/retained-seek-20261112/rounded-cargo-and-shortcuts/summary.json`.
The downloaded CI log is
`_build/qa/ci-test-batching-20261112/run-43206519259-linux.log`.
These local results do not claim a green subsequent full CI run.
