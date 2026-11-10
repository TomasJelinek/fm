# Shared active-panel toolbar qualification

The application now has one toolbar above both panels. It follows the active
panel and retains navigation/history, new file, attributes, Find, Filter,
comparison, Hot Paths, view and sort. Checksums, Folder Sizes and Properties are
visible actions. Drives, paths and tabs remain per-panel, and the existing F-key
bar retains copy/move/delete. The original checksum SVG is also used by the menu
and the configurable Ctrl+F8 footer slot.

Shared controls capture the pane plus its typed input context and model revision.
An open popup or held pointer cannot adopt a different pane, including when both
panes have the same numeric revisions. Hot Path keyboard selection validates the
opened context synchronously, before deferred property-change handlers run.
Native Winit touch capture happens before Slint's delayed Flickable forwarding.
Native cancellation invalidates pointer commits before the pinned runtime emits
its synthetic release; the same guard covers the footer. A new press resets the
guard, and keyboard/accessibility activation is independent of canceled input.

The toolbar scrolls horizontally with explicit overflow buttons. Compact windows
keep the 320×200 minimum: the active filter remains marked in the Filter tool,
and configured custom commands move into a context-bound popup using the existing
command controls. Their dedicated row remains at ordinary sizes. No dependency,
preferences schema, filesystem operation or backend action contract was added.

## Source and test scope

Production: `ui/shared-toolbar.slint`, `main.slint`, `file-pane.slint`,
`controls.slint`, `navigation-controls.slint`, `function-bar.slint`,
`function-button.slint`, `main-menu.slint`, `action-icons.slint`, `icons/checksums.svg`,
`src/scroll_input.rs`, and `src/presenter/function_slots.rs` in `midnight-ui`.
The new native-touch regression is included from `scroll_input_pane_tests.rs`.
Asset provenance and the keyboard reference describe the final controls. Other
pre-existing changes in these mixed worktree files are outside this toolbar task.

New actual-widget tests in `tests/shared_toolbar.rs` cover one action set, active
pane dispatch, same-revision popup invalidation, synchronous Hot Path invalidation,
native/selection gates, and 320×200 desktop/touch layouts with 32px fonts, an active
filter and configured custom commands. The native touch regression covers a pane
switch, location change and cancellation before/after the 100ms forwarding delay,
then verifies the next mouse, keyboard and touch inputs and footer cancellation.
Existing layout/navigation tests were adapted to the single toolbar and share
the overflow helper `tests/support/toolbar.rs`.

The coordinated Windows gate includes the library tests and these integration
targets: `shared_toolbar`, `comparison_toolbar`, `small_window`,
`compact_controls`, `panel_activation`, `panel_toolbar`, `panel_chrome`,
`panel_view_modes`, `commander_edges`, `panel_refinements`,
`interaction_controls`, `command_menus`, and `archive_options`. The last includes
the separately implemented archive scrolling fix and its three tests. Tests run
serially; artifacts stay in `_build/goal/shared-toolbar/`.

## Evidence ledger

- `red.log`: two actual-widget failures against the prior toolbar, proving missing
  shared controls. `red-facade-build.log` and `red-test-build.log` record the direct
  build against existing generated bindings.
- `touch-red.log`: one native-event failure, with a touch started on the left
  dispatching to the right after the active pane changed.
- `touch-cancel-red.log`: one native-event failure, with a canceled short touch
  dispatching to the unchanged pane. This confirmed the runtime cancellation
  boundary independently of pane retargeting.
- `intermediate-test-compile-error.log`: integration test compilation failed on
  an incorrect VRc borrow in the new geometry assertion. No test results are
  attributed to that invocation; the harness now uses ItemTreeRc::borrow_pin.
- The initial Slint parse reported redundant inner width/min-width and
  height/min-height declarations. Those declarations were removed before the
  final build.
- `preview-comparison.log`: both existing comparison toolbar tests passed in an
  already-built pre-cancellation binary. Root inspected wide and narrow touch
  screenshots under `preview/`; this is layout evidence, not final source proof.
- `windows-green.log`: the combined gate passed all 506 library tests, including
  the native touch regression, then passed two archive tests and failed the
  last-checkbox wheel/resize test. Cargo consequently did not run its remaining
  integration targets; those freshly compiled executables were run separately.
- `windows-panel_refinements.log`: the existing geometry test still expected the
  earlier 110% file-icon size, despite the worktree's already-implemented policy
  of one pixel of padding within each row. Its explicit expectations now use
  20px/38px while retaining the centered/within-control checks. The direct rebuild
  and passing rerun are `panel-refinements-build.log` and
  `windows-panel_refinements-green.log`; no production icon geometry changed here.

## Final Windows toolbar gates

All 12 toolbar/navigation integration targets passed, 44 tests total:
shared toolbar 5, comparison toolbar 2, small window 4, compact controls 1,
panel activation 7, panel toolbar 1, panel chrome 2, panel view modes 1,
commander edges 9, panel refinements 1, interaction controls 10, command menus 1.
Each has a `windows-<target>.log` under the artifact directory; panel refinements
uses its explicit `-green` rerun noted above. Together with the 506 library tests,
these validate the final interaction behavior. The final F10 menu artwork mapping
was changed after those bindings were generated; the all-target strict Clippy
gate regenerated and checked that final source (`windows-strict.log`).

Owned Rust formatting and whitespace checks passed. The source-size gate passed
all 1,240 handwritten files. `files.json` records the owned current source hashes;
it is a worktree snapshot, not a claim that mixed files contain only this task's
changes. The actual 320×200 touch/font32 screenshot and ordinary-width screenshot
were inspected: the file row remains visible, overflow actions are reachable and
the dedicated custom-command row returns at normal size. Artifacts are
`render/shared-toolbar-tiny-0.bmp`, `shared-toolbar-tiny-1.bmp`, and
`shared-toolbar-wide.bmp`.

Read-only review found no remaining toolbar or cancellation-boundary defect.
Archive scrolling, Linux gates, and integrated app qualification are appended
after their terminal results; none is inferred from the toolbar-only passes.

## Final archive/layout integration

The archive failure was a real large-font extent error in nested repeated rows,
not a missing animation wait. Explicit row heights account for the preferred
height of wrapped descriptions and custom extension names. The separate
[archive report](2026-11-08-archive-options-scroll.md) preserves the causal probes
and final geometry checks.

After that fix and the final F10 artwork change, all 14 focused tests passed on
both Windows and Linux: archive options 3, comparison toolbar 2, shared toolbar 5,
and small window 4. Windows proof is
`_build/qa/archive-scroll/windows-final.log`; the final strict all-target UI check
also passed (`windows-clippy.log`). Linux proof is
`_build/goal/viewer-opening-toolbar/linux/main-controls-final.log`.
Root inspected the final Windows large-font archive render as well as the
320×200 desktop/touch toolbar renders and the ordinary custom-command row.
Linux's headless CJK font coverage is not claimed as visual qualification;
its Unicode values, control targeting and removal checks pass.

Linux also passed the native-touch regression (1 passed, 501 filtered), including
cancellation and subsequent input. All 596 captured input hashes matched before
and after the final Linux gates. Logs and artifact hashes are in
`_build/goal/viewer-opening-toolbar/linux/verification.md`; this is focused Linux
UI validation, not a native Linux application or full-workspace qualification.

## Integrated Windows build and startup

`cargo build --locked --offline -p midnight-explorer --bin me` completed in
4 minutes 9 seconds with development debug information/incremental compilation
disabled and two jobs. `_build/debug/me.exe` is 82,165,248 bytes, SHA-256
`34024314756931066af264c5405f5406e1ba084c4e264932c8c1e08d9dfa6ddd`.
The app build reports eight existing unused root-seed warnings from the separate,
unfinished inline-tree integration. They were not suppressed; the scoped UI lint
passes above are not a claim that the entire in-progress workspace is lint-clean.

The new executable and the existing verified five viewer packages, native/archive
filesystems and thumbnail workers were staged under `_build/goal/shared-toolbar/`
and copied with SHA-256 readback to the protected runtime
`C:\Users\tjelinek\MidnightExplorer-20261108-100543-7c516b26`.
The matching build hash is registered, so `_build/debug/me.exe` resolves these
packages without weakening plugin path checks. Existing installations remain.

The actual `_build/debug/me.exe --ui-smoke-test` passed with owned Unicode file
fixtures and in-memory test preferences. Exit code was zero, stderr empty, and
`native-startup.png` was captured and inspected: one shared toolbar, per-panel
drive/tab/path controls, native icons, file listings and the complete footer are
visible. The preferences hash before and after was unchanged:
`bcf7388cc3c3e2742fd86febf6f91a3edac29fb312fdfc6bc054492bcbbab053`.
Evidence is `app-build.log`, `deploy.log`, and `native-startup.json` plus its
stdout/stderr logs in the shared-toolbar artifact directory.

The real PDF regression was repeated against this new runtime: 1 passed,
73 filtered, 11.06 seconds (`pdf-new-runtime.log`). It checks initial Fit,
enlargement, resizing and exact manual zoom across differently sized pages.
This final build is a debug iteration; the release executable measured by
[cargo-bloat](2026-11-08-cargo-bloat.md) was not replaced.
