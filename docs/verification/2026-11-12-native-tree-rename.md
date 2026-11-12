# Native tree Rename — 12 November 2026

## Scope and cause

Expanded native panels reached the general tree-operation refusal before F2 could
prepare the focused item. The presenter RED emitted zero commands instead of one typed
capture. F7 was already a pane-directory action and remains so. Salamander's
`CFilesWindow::RenameFile` (fileswn5.cpp) likewise uses the caret rather than marked
selection and rejects parent navigation; the implementation here uses this repository's
typed provider and prepared-mutation interfaces, without copying its native UI code.

The new `TreeRenameRequest` binds exact projection/request/row identity. The native-only
capture hands an opaque `CapturedRenameTarget` to the existing bounded operation worker.
Capture retains the provider-resolved path and registration, not a filesystem snapshot.
Native object identity is pinned by `MutationService::prepare`; the replaced-source
regression changes the filesystem after that preparation. Preview, confirmation, invalid-name
handling, cancellation and versioned recoverable replacement retain their existing behavior. Four cleanup slots are derived from the
same named four-slot capture limit, using the existing shared retirement domain.
Prepared pins precede capture ownership; native target/registration destruction precedes
capture admission release. The final capture guard works after the Tokio runtime and
last domain handle disappear.

The implementation does not enable archive writes, create new descendant file rows,
or change F7's pane-directory destination. Navigation before admission cancels capture;
after admission the pinned original target remains authoritative. Tree completion uses
existing loaded-branch maintenance. Native changes outside the pinned target's identity
contract are not a snapshot of the whole displayed subtree.

## Regression evidence

Local logs are under `_build/qa/tree-rename-20261112/`.

- `ui-red.log`: F2 presenter assertion failed as intended, zero commands versus one.
  `ui-red-build-migration.log` preserves the earlier unrelated configuration-migration
  import error; that compile did not count as a regression result.
- `linux-capture-red.log`: four focused capture cases failed at the deliberate unsupported
  baseline seam; build 2m32s, test execution 0.14s.
- `linux-rename-green.log`: 11 focused cases passed, build 1m17s, tests 0.22s. Covers exact
  focused descendants despite other marks; native kinds; forged, unacknowledged, replayed,
  parent and cancelled capture; retained provider/capture capacity; unsupported archives;
  confirmation/cancellation; real native file/directory rename, replaced source refusal
  and recoverable collision replacement.
- `linux-retirement-green.log`: one additional queued/cancelled capture ownership case
  passed, build 22.41s, tests 0.10s. The last queued source is dropped without Tokio or a
  domain owner while a real mounted filesystem/File destructor is gated. Cleanup runs
  off the caller, and the shared capture permit remains occupied until cleanup finishes.

- `f4-retirement-red.log`: the adjacent F4 audit regression failed as intended: after a
  captured editor result met a saturated launch queue, `retire_all` left it owned by the
  presenter and final destruction occurred on the same caller ThreadId(2). Build 7m20s,
  one assertion failure in 0.01s. `cancel_tree_editor(None)` is now included in retirement.

- `ui-green.log`: 31 presenter/tree tests passed in 0.08s after a 3m08s build,
  including the F4 final-owner regression and Rename focus/retry/stale/refresh cases.
- `cmd-form-green.log`: the parent's requested Cmd picker regression passed, one test
  in 0.00s, using the same feature union.
- Initial `ui-keys-green.log`: both installed F2 routes passed; F7 was refused before
  preparation because the shared no-watch fixture omitted `PaneCommitted`. The app's
  `VisibleSync` sends that exact snapshot independently of watcher setup. The test now
  mirrors that acknowledgement before key input; backend snapshot checks are unchanged.
  This failure is fixture evidence, not a claimed F7 production defect.

Final corrected key-routing qualification passed all 3 cases on Windows in 2.12 s
(`ui-keys-final.log`) and Linux in 1.78 s (combined gate). Both F2 panel routes and
F7's acknowledged pane-directory destination are covered. The independent source
review found no actionable reservation, native-pin/capture destruction-order or pending
Rename retry issue. No broad provider-lifetime completion claim follows from this change.

## Commands

Linux uses the existing `_build/qa/interaction/linux-verification-env.sh`,
`_build/qa/elevation-linux-20261111/sdk.toml`, native private TMPDIR, target `_build/linux`,
locked/offline Cargo and two jobs. The focused filters are `tree_rename` and
`queued_cancelled_rename_capture` on `midnight-engine --lib`.

Windows uses the established seven-package UI feature union, pinned worker SDK config,
`_build` target and locked/offline Cargo. Exact scripts and logs are retained alongside
the results. The [combined checkpoint](2026-11-12-config-rename-cmd-checkpoint.md)
records the completed Windows/Linux engine, affected UI and strict lint gates,
plus startup smoke checks of the newly built executable with native and software
renderers. These checks do not qualify every physical desktop/GPU combination.
