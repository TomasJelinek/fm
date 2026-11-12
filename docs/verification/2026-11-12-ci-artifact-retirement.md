# Windows CI artifact retirement — 12 November 2026

Run [43117240558](https://github.com/TomasJelinek/fm/actions/runs/43117240558)
for `a6d90f3ac006fbed88bb34764e4dfee6c4df41e8` failed in Windows cleanup.
The `panel_search` test passed at 23:47:55.531 UTC on 11 November; approximately
68 ms later, removing its reported generated executable raised WinError 32
(sharing violation). The Test step had begun at 23:12:10 UTC. This was neither a
failed application assertion nor evidence that all subsequent CI gates passed.
The separate Ubuntu compact-Find fixture failure was corrected in `35227750`.

The runner now retries only Windows sharing/lock violations 32/33, with a
monotonic two-second budget and sleeps no longer than 100 ms. It still validates
the complete Cargo-reported successful artifact set before deletion. Each attempt
rechecks the same pending path, canonical containment, regular-file type and
captured device/inode identity. Substitution fails closed; permission denied and
other errors fail immediately. Exhaustion reports the original sharing error.
No new candidates are discovered during retry, and no failed test artifact is
removed. Per-gate cleanup retains the existing disk bound.

This is constrained cleanup of trusted generated CI artifacts. The checks do not
make path-based unlink atomic or eliminate its inherent check/use race. The
budget is a chosen retry ceiling; the original lock holder and duration were not
identified. No permission policy, application code, dependency or Cargo setting
changed.

## Local evidence

- RED: the new transient regression failed against the previous single-unlink
  implementation for both 32 and 33.
- Windows focused module: 21 tests, 19 passed and two symlink-privilege skips.
- Linux focused module: all 21 passed, including the symlink substitution cases.
- Tests cover transient/persistent locks, original-error preservation, replacement
  files/directories/symlinks, other permission errors and non-Windows behavior.
- A real Windows CreateFileW handle denied delete sharing on an owned fixture.
  Releasing it after 150 ms allowed retirement to finish in 202.688 ms; only that
  generated artifact was removed. Before the fix, the same held handle produced
  WinError 32 and retained the artifact.
- UTF-8/LF, file-size and diff-whitespace checks pass. Root reviewed the change
  with no blocking findings. No independent commit was made by the implementing
  agent.

Evidence is retained under `_build/qa/viewer-settings-20261111/`:
`ci-a6d90f3-windows-job130850286183.log`, `ci-retire-red.log`,
`ci-retire-windows.log`, `ci-retire-linux.log`, `ci-retire-native.py`, and
`ci-retire-native.log`. Focused command:
`python -B -m unittest scripts.tests.test_ci_workspace_tests -v`.

At this verification handoff, the subsequent exact-head
[35227750 run](https://github.com/TomasJelinek/fm/actions/runs/43122031929)
finished with 18 successful prerequisite jobs and two failed host jobs. Ubuntu
failed the monospace font-catalog regression; its exact reproduction and local
correction are recorded in [monospace evidence](2026-11-12-monospace-catalog.md).
Windows job `130866843153` again passed its last application test, this time
`tabs_bookmarks` (1/1 at 00:30:27.132 UTC), then failed to retire its executable
with WinError 32 at 00:30:27.203 UTC, approximately 71 ms later. The full log is
`_build/qa/viewer-settings-20261111/ci-3522775-windows-job130866843153.log`.
That pushed head contains neither local correction. The pipeline is failed; the
corrections' eventual CI results remain unqualified until a new head completes.
