# Compact copy stages and cancellation

Scope: the owned temporary destination of Copy and cross-volume Move. Same-volume
native Move performs rename and does not create a partial content copy.

## Windows native boundaries

The compact-name regression first failed against the previous 47-character name.
Stages now use `.me-` followed by sixteen random hex characters (20 total).
Exclusive native creation and retained directory/file handles remain unchanged.

Real filesystem tests cover:

- A Unicode destination parent longer than 280 UTF-16 code units, verified
  publication to a one-character final name, and unchanged source bytes.
- Actual exclusive-create collisions, bounded retries and preservation of the
  existing colliding file.
- Cancellation after a partial write and during independent readback, with no
  stage name remaining and existing target/source bytes preserved.
- Removal of the stage name while an external delete-sharing reader still holds
  the bytes; namespace deletion is distinguished from physical storage lifetime.
- A real read-only cleanup refusal, its exact residual path, and successful
  cleanup after restoring only the owned fixture's original permissions.
- Both stage factories failing after creation. An initial cancellation must not
  hide a subsequent native cleanup refusal: the cleanup error retains its native
  kind and residual path, with the original failure appended as context.

The last case has a causal failing test showing `Interrupted` masking
`PermissionDenied`, followed by all five focused native tests and strict
all-target Clippy passing. The earlier full platform run passed 166 tests with
11 existing opt-in tests ignored. Native staging, transfer and metadata integration
tests also passed. Evidence is under `_build/qa/compact-staging/`.

## Linux native boundaries

Four additional real filesystem tests use anonymous `O_TMPFILE` stages, Unicode
destination paths longer than 800 bytes, partial cancellation under both checksum
policies, explicit cleanup, destructor fallback and verified publication. They
preserve existing/completed files and unfinished source bytes. No Linux production
fix was needed. The platform all-target run passed 200 tests with 14 existing
opt-ins ignored; strict Clippy passed. Logs are
`_build/qa/native-fast-move/linux-stage-cleanup*.log`.

## Limits

Cancellation waits for an in-flight native I/O call to return. It cannot interrupt
an operating-system call by freeing its buffers or killing the worker thread.
An unavailable device, changed permissions or another native refusal can prevent
cleanup; that error must remain visible rather than being described as successful
removal. Completed files are not rolled back. These tests exercise owned fixtures,
not user data, and do not simulate sudden power loss or a killed process.

## Shared queue and presentation

Causal failing tests exposed rejected constructors dropping a stage without
explicit cleanup, cancellation hiding real native errors, metadata aggregation
retaining `Interrupted` instead of a cleanup refusal, and bounded error history
discarding a partial file's residual path. Fixes retain the cleanup error's native
kind/path and original context. The active object's cleanup and drop still happen
in the original order; only insertion of its diagnostic is deferred until later
metadata/restoration cleanup finishes. Error counts increment once and omitted
details are disclosed.

The actual queue matrix covers 24 scenarios: Copy and forced cross-volume Move,
full SHA-256 and sampled verification, cancellation during blocked Write/Sync/
Readback, and new/existing destinations. It passed on Windows and Linux. The
five cleanup-contract regressions also passed, including 17 later cleanup
failures after an active partial copy.

Final frozen-source tests passed:

- Windows engine: 746 passed, 7 existing opt-ins ignored.
- Linux common backend: 175 passed across 27 binaries; core: 51 passed;
  engine: 737 passed, 7 existing opt-ins ignored.
- UI library: 786 passed. Its cancellation regression checks Copy and Move,
  preserves the completed-file count, and shows the exact Unicode residual in
  the last of 16 retained error slots.
- Six path-elision integration tests passed after correcting a stale test fixture
  field name found by all-target compilation; no Find production behavior changed.

Strict all-target Clippy passed for the Windows UI and Linux core/common/engine/
platform scopes. Workspace formatting and diff checks passed; all 1,751 checked
handwritten source files fit the 100,000-byte limit. Independent review found no
remaining issue in this change's ownership, cancellation or diagnostic handling.
No application dependency or plugin ABI changes were introduced.

## Debug executable and startup

The final Windows debug build completed successfully. The executable was copied
with independent SHA-256 readback into the existing protected installation and
its matching-build plugin registration refreshed. Both installed and build-path
executables have SHA-256
`C5BA35E6B98D632F4716200E4395269917922B24B8B345CB4FB62A32B99C5ED3`.

The initial startup probe inside the coding sandbox failed because the sandbox
denied reading installed-plugin security descriptors. The same executable passed
both startup and actual Windows UI smoke tests in the ordinary user context;
security checks were unchanged. The UI probe used isolated in-memory preferences.
Its rendered snapshot was inspected: both populated panels, path bars, status
lines, command line and bottom actions are visible, without a startup error.

Paths: `_build/debug/me.exe` and
`C:\Users\tjelinek\MidnightExplorer-20261110-html\me.exe`.
Logs/result JSON and the inspected `native-startup-acf1f7bb58784e36b29a2c7604603c08.png`
are under `_build/qa/operation-speed-charts/`; final build/test logs are under
`_build/qa/compact-staging/`. This is debug qualification, not a new release or a
claim that the complete workspace/platform/feature backlog is finished.
