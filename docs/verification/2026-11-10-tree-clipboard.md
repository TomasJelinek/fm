# Native inline-tree clipboard implementation evidence

Ctrl+C/Ctrl+X source routing now captures exact acknowledged native tree
occurrences, preserving Copy/Move intent through native publication. Ctrl+V uses
the native panel root and the existing verified queue/Move confirmation. Archive
and mixed-provider exports remain Unsupported; no display or extraction paths
are exported. See the [contract](../developer/TREE_PANEL.md#native-clipboard-from-expanded-panels).

## Windows evidence collected so far

Logs under `_build/qa/tree-clipboard/` record:

- `ui-red.log`: the original nested Copy and Cut paths both failed their intended
  admission assertions; existing tree Paste-to-root behavior passed.
- `backend-capture-red.log`: five expected missing-capture failures and one
  passing archive refusal case before the implementation.
- `backend-worker-red.log`: four expected publication/intent/validation/retirement
  failures before implementing the captured native clipboard route.
- `backend-green.log`: 29 focused engine tests passed, including native Copy/Cut
  intent, exact selection/focus, archive refusal, shared capture capacity,
  cancellation after native open, provider invalidation, bounded queue refusal,
  retained source lifetime and off-dispatcher shutdown retirement.
- Historical `backend-full.log`: core 49 passed; engine 656 passed, two failed and one opt-in
  benchmark was ignored. The failures were directory-size runtime deadline tests
  `overlapping_queued_root_keeps_running_after_first_finishes` and
  `repeated_fast_scans_respect_one_map_per_quarter_second`. That run does not
  establish a green full engine suite. The later successful library gate below
  supersedes those failures for the current source.

Current qualification logs:

- `_build/qa/settings-polish/windows-libs-green.log`: core 51 passed, engine
  670 passed with seven opt-in tests ignored, and UI library 747 passed. These
  include the presenter clipboard regressions and retained-capture tests.
- `_build/qa/settings-polish/operation-final-green.log`, the `tree_clipboard`
  target: five passed. Installed Slint keyboard callbacks drive real native
  Copy/Move with nested Unicode paths in both panel directions, explicit Move
  confirmation, verified destination bytes, source retention/removal, parent
  refusal and focused-item fallback. Later external writes increase the native
  watcher callback count and reconcile both retained tree sessions. The OS
  clipboard boundary is replaced with an in-memory service whose thread affinity
  and teardown are asserted; this is not native clipboard interoperability proof.
- The final operation log also passes tree Copy two tests, tree Move six tests,
  and the focused-close Enter regression. That regression first failed, then
  verified that explicit X focus cancels rather than confirming deletion.
- The earlier `windows-focused-green.log` retains failures from the separate
  `tree_bulk_selection` fixture. Its corrected four-case rerun passes in
  `tree-bulk-final.log`. `integration-results.json` records the latest run of each
  of 30 selected UI targets: 127 tests passed without counting repeated runs.
- `_build/qa/plugin-options/full-backend-linux.log`: core 51 passed and engine
  661 passed with seven opt-in tests ignored. This qualifies the Linux backend
  source, not a Linux UI bridge or desktop clipboard session.

The capture retention cleanup keeps the provider registration and admitted
selection snapshot alive explicitly. Selection revisions are still checked at
admission; the unused duplicate scalar was removed from the retained batch.

## Final review and Windows artifact

Scoped strict Clippy for core/engine/UI libraries, workspace formatting,
whitespace and source-size checks passed. Independent review of the shared
operation controls found and corrected the focused-X Enter issue before the
final operation gates and deployment.

The Windows debug build and deployed private runtime were read back with the
same SHA-256:

`E71ED919E0B0EAE14DEC5B3062213F9DA6978F287206AC849CB20FA0BCAC6E11`

Backend and native Windows UI startup checks both exited zero with empty stderr;
the captured native screenshot was reviewed. Exact artifact paths, logs and
smoke scope are recorded in the
[settings/panel qualification](2026-11-10-settings-polish.md#exact-windows-artifact).
This establishes the described Windows debug deployment, not a published release.

## Remaining qualification

Run the Linux UI bridge and installed native OS clipboard interoperability before
declaring this slice qualified on both platforms. Existing native clipboard
services are reused without ABI or dependency changes. No complete TREE-01 claim
follows from this native clipboard slice.

The separate yellow clickable size-warning feature has only standalone report
storage tests so far; scanner/core/UI integration remains open in its
[plan](../superpowers/plans/2026-11-10-size-warning-reports.md).
