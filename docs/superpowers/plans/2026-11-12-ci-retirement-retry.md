# CI artifact retirement retry implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to
> implement this approved narrow task. Steps use checkbox syntax for tracking.

**Goal:** Let a successful Windows test executable survive a brief sharing lock
without aborting the complete CI suite or weakening artifact validation.

**Architecture:** Keep per-gate retirement and its complete-set validation. Retry
only Windows sharing/lock violations (32/33), for at most two seconds with sleeps
of at most 100 ms. Before each attempt, revalidate the same path, containment,
regular-file type and original `(st_dev, st_ino)` identity. Preserve the first
sharing error at exhaustion; propagate other errors immediately.

**Tech Stack:** Existing Python standard-library CI runner and unittest fixtures.

**Spec:** The coordinating task approved the scope below after the exact Windows
CI failure and a native handle-lock reproduction; implementation is authorized.

## Approved scope and approaches

The completed test passed, then artifact retirement failed with WinError 32.
A generated local fixture reproduced that failure while a native Windows handle
withheld delete sharing; the same retirement succeeded after the handle closed.
Immediate bounded retry preserves the runner's per-gate disk bound. Deferring
cleanup until the suite ends would accumulate its large executables; silently
ignoring failed cleanup would hide persistent errors. Neither alternative is used.
The two-second budget is an implementation choice, not an observed lock duration.

## Global constraints

- Change only the CI runner, its focused tests and this verification plan.
- No dependency, application, UI, engine, permission-policy or Cargo change.
- Preserve full-set validation before the first deletion; no glob/recursive delete.
- Never discover new cleanup candidates during retry or retry failed test artifacts.
- Non-Windows platforms and non-sharing errors keep immediate failure behavior.
- No commit: root reviews and integrates the qualified change.

## Review focus

- A substituted regular file retains its replacement bytes and fails closed.
- A symlink/directory substitution cannot redirect cleanup.
- Repeated lock errors stop within the monotonic budget and preserve the first error.
- Permission denied and non-Windows failures are never treated as transient sharing.
- Successful retry removes only the originally reported executable/PDB set.

## Task: bounded retirement retry

Files: `scripts/ci-workspace-tests.py`,
`scripts/tests/test_ci_workspace_tests.py`.

- [x] Add deterministic transient 32/33, persistent lock, substitution, permission
  and other-platform regressions, using an injected clock/sleep through mocks.
- [x] Run the new transient regression against the current runner and record RED.
- [x] Add original file identities and a narrow monotonic retry helper; retain all
  existing gate, path, complete-set and test-success checks.
- [x] Run the complete focused unittest module on Windows and Linux; run an owned
  native Windows hold/release fixture through the corrected runner.
- [x] Check LF/whitespace, record exact results, and request root review. Continue
  monitoring exact-head CI separately; local passes do not make that run green.

## Verification ledger

- Base: `35227750df8bce15c6419310c9307ab9386604a3`; no shared task interfaces.
- RED: the new transient regression failed for both WinError 32 and 33 with the
  existing single `path.unlink()` call (`ci-retire-red.log`).
- Ruling: retain the existing shared checkout and `_build/qa` artifacts as the
  coordinating task requires; do not create a worktree or commit independently.
- GREEN: focused module ran 21 tests on Windows (19 passed, two symlink
  privilege skips) and Linux (21 passed, no skips). Logs:
  `_build/qa/viewer-settings-20261111/ci-retire-{windows,linux}.log`.
- Native Windows hold/release: an actual CreateFileW handle denied delete sharing,
  then closed after 150 ms; retirement succeeded in 202.688 ms and removed only
  its generated fixture. Log: `ci-retire-native.log` in the same QA directory.
- The Windows CI failure log is `ci-a6d90f3-windows-job130850286183.log`.
  The failure followed a passing panel_search test; no test assertion was skipped.
- LF/whitespace checks passed. Root review and integration remain pending; the
  running GitHub head does not yet contain this local correction.
