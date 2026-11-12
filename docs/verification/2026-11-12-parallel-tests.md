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
