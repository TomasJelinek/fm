# Bounded Cargo integration-test batching

## Scope and baseline

Starting head: `81659a7`. The preceding runtime investigation observed a complete successful Windows
Test step of 50 m 32 s for `6f06ec4`; shorter failed jobs are not full-suite baselines. See
[the original runtime evidence](2026-11-12-ci-windows-test-runtime.md). This change does not claim a
hosted full-suite speedup from command counts alone.

The runner keeps Cargo as executor. Repeated `--test` selections combine only integration gates;
library, doctest, binary, compile-only examples and test-enabled examples retain their prior commands.
The platform-active feature union, explicit eligibility checks, environment, package working directory,
custom harnesses, target runners and runtime library lookup remain Cargo's responsibility. Cargo's
[documented test selections and execution](https://doc.rust-lang.org/cargo/commands/cargo-test.html)
support repeated selectors and serial executable execution. No workflow, timeout, test-thread or
ignore/failure policy changes are included.

## Bounds and failure behavior

Batch size counts selected `(package_id, target_name)` executables. Same-name groups remain atomic
because workspace Cargo selects the name across members. The bound is the larger of the configured
batch size and the largest existing same-name group; this exception already exists at serial size one.
Within a batch Cargo can order tests differently from the previous global alphabetical target-name
order. Tests must not depend on another executable's prior side effects.

Retirement first validates every expected pair, unique executable, generated path and optional PDB.
Missing, duplicated, additional or mismatched artifacts fail closed before any deletion. Existing
containment, symlink, file identity and narrowly bounded Windows sharing-lock checks remain intact.
A failed Cargo batch retains all its artifacts and stops the runner; no later batch executes. A compile
failure can now precede tests of other targets in that same batch, consistent with Cargo fail-fast.
No failed artifact is cleaned merely because another test in the batch succeeded.

## Targeted fixture evidence

The new fixtures first failed against the serial implementation, which had no batching entry point or
multi-target identity model. The targeted Python suite ran **31 tests: 29 passed and two existing Windows symlink fixtures
were skipped** because this session lacks symlink creation privilege. Command:

```text
python -B -m unittest discover -s scripts/tests -p test_ci_workspace*.py -v
```

Raw logs: `_build/qa/ci-test-batching-20261112/python-red.log`, `python-green.log` and final
`python-final.log`. The approved default-eight regression first failed with nine serial commands, then passed with the expected eight-plus-one grouping; see
`default-red.log`. Tests cover unchanged non-integration commands, feature selection, same-name groups, bounded batches, failed-batch
retention, exact complete-set retirement, duplicate paths and Cargo JSON mixed with ordinary/custom
harness output. The new suite is included by the existing CI Python-test discovery command.

## Same-target paired measurement and default

The default is now **eight selected integration executables per command**, retaining the explicit
`--integration-batch-size 1` escape hatch and size four option. No workflow edit is necessary.

The native Windows measurement used the existing `.tools` toolchain, pinned SDK configuration,
`CARGO_BUILD_JOBS=2`, debug information disabled and incremental compilation disabled, matching the
workflow profile settings. It used four `midnight-core` and four `midnight-platform-common` targets:
`archive_associations`, `checksum_shortcuts`, `file_compare_shortcut`, `selection_ranges`,
`comparator_preferences`, `panel_split_preferences`, `transfer_preferences` and `viewer_preferences`.
The active workspace feature set came from native Cargo metadata plus the depth-zero feature tree.
The eight targets are independent of the simultaneous source-only engine work.

Windows and Linux compilation were paused for the measurement. One excluded warm-up took 24.591 s.
Paired order was **1, 4, 8, 8, 4, 1**; every sample ran the same **19 tests in eight targets**, with no
ignored/failed tests, and exact selected executable/PDB retirement between samples. Thus each measured
sample had a warm dependency graph but needed the same eight test harnesses relinked.

| Batch bound | Paired elapsed seconds | Mean seconds | Change from serial mean | Peak selected exe/PDB bytes |
| --- | --- | --- | --- | --- |
| 1 | 23.096 / 22.992 | 23.044 | baseline | 14,373,376 |
| 4 | 21.762 / 21.363 | 21.563 | 6.4% less | 46,851,584 |
| 8 | 19.267 / 20.455 | 19.861 | 13.8% less | 73,445,888 |

Cargo's summed reported build-profile time was 4.76–4.83 s at size one, 2.03–2.05 s at size four,
and 1.59–1.60 s at size eight. Cleanup took 0.012–0.120 s per complete eight-target sample. The rest of
the wall time includes Cargo/process launch and test execution; test harness body timers rounded to
0.00 s. These measurements do not separately identify OS process-start cost or rustc link time. The
benefit includes Cargo's ability to schedule multiple harness builds under the existing two-job limit,
not solely reduced Cargo startup.

The **current native graph** has 282 commands at size one: six unchanged non-integration gates plus
276 integration-name gates. Size four produces 81 commands (75 integration batches); size eight
produces 44 (38 integration batches). The largest same-name group has three executables. These exact
counts supersede the earlier 281/275 observation and the unverified estimate of 35 batches. Selection
and feature eligibility are unchanged; reducing command count does not remove tests.

Raw evidence lives under `_build/qa/ci-test-batching-20261112/`: `measure.py`, `measure.log`,
`metadata.json`, `feature-tree.txt`, `graph-counts.json`, `samples.json`, `summary.json`, and each
`sample-*-batch-*-gate-*.log`. The harness asserts identical target and test inventories. All seven
runs including warm-up completed successfully. The Python regression suite then passed with the
final default; no Cargo source change followed the paired measurement.

This is a **representative local 19-test/8-target result**, not a complete hosted CI comparison.
Selected-artifact peak is not whole-workspace disk usage, and these small core/platform harnesses do
not measure the largest generated UI binaries. The eight-target cleanup bound prevents accumulating
all integration artifacts, but hosted full-suite time and disk headroom still need observation on the
next ordinary CI run. No projected full-CI speedup is claimed. Per-gate Cargo elapsed and retirement
summaries now make that future run easier to attribute; no subprocess deadline was added.

## Independent review

The image-loading agent reviewed the final runner and fixture diff read-only and found no actionable
issue in unchanged non-integration selections, atomic name groups, exact pair/path validation,
failed-batch retention or mixed JSON output. The reviewer ran no tests and made no source edits.
