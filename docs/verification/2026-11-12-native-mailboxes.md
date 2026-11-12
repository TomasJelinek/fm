# Archive-size mailboxes and native branch shutdown

12 November 2026. Baseline `81659a7`. This batch continues the ownership audit;
it does not close LIFE-01 or the broader application/UI goal. Source changes add
two startup retirement leases for archive-size lane pending state. Separate archive
navigation work adds 135 slots and one intake waiter. The current total is
303 slots and seven waiters; the independent process limits remain eight and sixteen.

## Reproduction and ownership evidence

The actual-file-backed queued size-root test failed before the change: a simulated
native worker unwind left a root in the pending lane, and final controller release
closed its file on the caller. The corrected lane holds `Mutex<Pending>` inside
its pre-reserved worker-owned guard. Normal native draining and the existing queue,
coalescing and cancellation policy remain unchanged. No per-destructor worker or
executor-side cleanup fallback was added. A second test blocks actual native
cleanup and proves the pane's slot is unavailable until destruction finishes.

The tree completion probe passed before any change. Its completed rows and consumed
root registration do not retain a native mount. It still holds branch admission
until the completion is drained. A separate runtime-shutdown test submits the real
branch closure behind an occupied blocking worker, aborts it and shuts down Tokio:
the native owner closes on that worker and admission remains occupied while queued.
The production `start_branch` helper merely exposes the existing submission boundary;
no tree completion guard or extra cleanup capacity was justified.

`_build/qa/native-mailboxes-20261112/windows-red-confirmed.log` preserves the actual
Lane failure and the passing tree control. Archive navigation has its own RED/GREEN
record. The first focused final Windows artifact passed 22 archive tests, 32
archive-size-related tests and both tree lifetime tests; the archive stale-command
follow-up and combined integration checks are qualified separately below.

## Integration findings retained during qualification

The first combined Windows run passed 969 units and failed one stale archive command
case (seven existing ignored). An earlier navigation high-water check bypassed the
new per-action rejection guard. The correction retires that raw archive action at
both refusal points before continuing; unrelated stale navigation remains unchanged.
The exact original log is `windows-final-stale-rejection-failure.log`. Linux reproduced
the same case. The corrected seven archive lifecycle regressions then passed in 0.35 s.

The next complete Windows unit suite passed all 970 tests, but the existing
`retained_navigation_targets_share_one_pane_mount_budget_across_sessions` integration
failed. A proposed two-retained-mount bound confused native worker/process limits with
capabilities retained by consumers. The original integration intentionally retains 64
archive targets and still needs to admit a new root. Its assertion was not weakened.
`windows-final-retained-budget-failure.log` preserves this regression; Linux reproduced it.

The corrected retention policy derives 132 tokens from the existing two pane registries
of 64 mounts, two current unadopted roots and two in-flight replacements. Request guards
and serialized intake add three slots. External output holders share that explicit
aggregate retention budget; saturation keeps accepted outputs intact. This preserves
the existing registry capacity and replacement headroom instead of imposing the host's
two-process limit on all retained archive capabilities.

## Shared Delete progress and generated-file cleanup

Source review confirmed Trash and permanent Delete use the same `JobProgressWindow`
as Copy/Move, including discovery, items/s, ETA, pause and exact-operation cancel.
The existing `trash_and_permanent_delete_share_modeless_progress_and_close_means_cancel`
regression was rerun from its unchanged native-window harness: 1 passed in 0.38 s.
It covers independent Jobs history, both deletion modes and rejected/accepted close
admission. No second Delete layout was created. This is targeted shared-window
qualification, not a claim of new whole-desktop manual testing.

Validated obsolete release intermediates (`.fingerprint`, `build`, `deps`, `examples`,
`incremental`) under `_build/release` were removed after confirming containment and
absence of reparse points. This recovered 4,606,035,137 bytes (4.29 GiB); the release
EXE, plugins and current debug build were preserved. D: had 38.35 GiB free immediately
afterward. Concurrent later builds can change free space.

## Remaining boundary

Warning-report spools and repeated builder replacement remain separate ownership
follow-ups. Two real report-spool failures are reproduced on Linux; the
[report-storage evidence](2026-11-12-report-storage-retirement.md) preserves the
QA patch and next implementation contract. Temporary failing registrations were
removed and the original production file restored exactly before final gates. A tree metadata-only result must not be confused with a generated report
that really owns native file handles. The current
[engine contract](../developer/ENGINE_LIFETIME.md) and
[backlog](../IMPLEMENTATION_BACKLOG.md#lifecycle-follow-up-found-on-12-november)
retain that distinction. No plugin ABI or dependency/license changed.

## Final Windows source checks

After correcting both integration findings, the complete affected engine unit suite
passed **970 tests**, with seven existing opt-in tests ignored, in **6.55 s**. The
three affected integration executables passed **33 tests**, with one existing native
plugin installation fixture ignored:

| Integration | Passed | Ignored | Test-body time |
| --- | ---: | ---: | ---: |
| archive_selection | 3 | 0 | <0.01 s |
| tree_controller | 19 | 1 | 0.44 s |
| tree_filters | 11 | 0 | 0.49 s |

Strict Windows engine all-target Clippy passed in 19.49 s. Exact logs are
`windows-final-tests.log` and `windows-clippy.log` in this report's QA directory.
The production source limit check passed for 2,050 handwritten files (at most
100,000 bytes each). `git diff --check` passed. Earlier failed logs remain alongside
the corrected evidence; no test or assertion was removed to obtain these passes.

CI runner qualification is independent: the [batching report](2026-11-12-ci-test-batching.md)
records 29 passing Python tests and two existing Windows symlink-privilege skips,
plus the measured 13.8% representative local improvement. Hosted full-suite timing
remains unmeasured for the new default-eight batching policy.

## Final Linux, debug deployment and startup

The corrected Linux build passed **66 selected engine unit cases** across archive
navigation, archive sizes, tree lifetime and engine lifecycle, plus **47 integration
cases** (one existing native installation fixture ignored). Strict engine all-target
Clippy passed in **2 min 08 s**. The archive owner's
[verification record](2026-11-12-archive-navigation-retirement.md) links exact binaries,
hashes and per-filter timings. The completed tree/shutdown controls pass on both OSes.

Windows debug app/elevation build completed in **1 min 18 s** with two pre-existing
vendored femtovg warnings. App `_build/debug/me.exe` SHA-256:
`1fa6ee37f6023e2f53770e25d4b0286e3b318fe34841dacffd00cc2dad1b76fe`.
Private plugin runtime: `C:/Users/tjelinek/MidnightExplorer-20261112-mailboxes-1fa6ee37`, registered against this exact build hash.
Previously qualified plugin payloads were reused; no plugin source or ABI changed.
The deployment verified private ownership and hashes, then removed its validated
owned staging copy, recovering another approximately 0.532 GiB.

All three fresh app startup cases passed with exit zero and empty stderr: backend
navigation (Git/bin), native-renderer UI startup (System32/repository) and software
UI startup with the same panels. User preferences retained their exact hash.
`final-native.png` and `final-software.png` were inspected: both populated panels,
paths, toolbars, command line and bottom function row render without an error panel.
These are startup/render checks, not a claim of exhaustive desktop interaction or
smooth scrolling qualification. Raw logs, screenshots and deployment script are in
this report's QA directory. `cargo fmt --all -- --check` also passed on final source.

A final independent read-only review found no actionable issue in the corrected
retention bound, both stale-command exits, pending intake ownership or the two-pane
regression. All changes in this batch preserve operation authority and checksum policy.
The report-spool and remaining feature/UI backlog stays open; this report does not
claim the broad goal complete.
