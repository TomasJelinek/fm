# Dialog polish and native informational windows

Status: this UI polish pass is implemented and verified on Windows. The separate
remaining native-window migrations are still pending.

## Scope

Reviewed General, Appearance, Keyboard, Editors, Plugins, Previews, Archives,
Colors and Custom Commands; Rename/Create/Delete/Copy/Move and conflicts;
Attributes/Times/Case; Find; Checksums; Jobs; Make File List; Hot Paths; panel
filter and directory comparison; Help/About/Notices; viewers and file comparison.
The shared design contract is [Dialog ergonomics](../developer/DIALOG_ERGONOMICS.md).

## Changes

- Options starts at a smaller 740×520 logical-pixel footprint, scaling for large
  fonts and clamping to its current host. Narrow pages stack fields instead of
  forcing horizontal canvases. Field instances, caret and drafts survive resize.
- Shared wrapping checkboxes keep native keyboard/accessibility semantics and
  full-row touch targets. Long labels wrap in settings and operation forms.
- Argument, command, plugin, color and keyboard lists size to actual content.
  Keyboard single-result borders no longer clip rows; selected rows remain
  visible after font-size changes. Empty color instructions get wrapped height.
- Operation forms and notices choose initial heights from wrapped content.
  Longer diagnostics scroll; short summaries avoid large empty space. User
  resizing remains independent. Hot Paths stacks its shortcut controls and
  sizes its list to saved paths.
- Find starts with its editable mask; compact idle windows show that field.
  Running compact searches release query space to results. Checksum page and
  algorithm controls share the available width. Jobs scrolls its complete
  chart/diagnostics while keeping Close reachable, including keyboard Escape.
- Find does not reserve space for hidden status/query presentations. Its empty
  state cannot overlap the sorting header. Attributes/Times/Case keeps its one
  shared recursion checkbox inside the scrolling body, so large fonts cannot
  push Cancel outside a minimum-size window.
- Scrollable settings and Find fields reserve the overlaid scrollbar width;
  right-edge actions stay clickable. Empty hints do not create blank tooltips.
- Overflowing operation footers retain a stationary, visibly focused primary
  action. Return/Space confirm, Escape cancels, and pointer/accessibility input
  uses the same action even after the footer scrolls. Cancel keeps its own
  keyboard behavior.
- Viewer/comparator UI metrics follow the configured UI font. Comparison action
  buttons use their caption width instead of stretching across the toolbar.
  Document fonts remain independent; status lines are vertically centered.
- Directory captions borrow unused byte-size space without moving metadata;
  explicit anchoring prevents overlap with icons. Parent entries use an up arrow.
- Help/About/Notice use independent native windows. Shared native ownership,
  focus, session and modal-lease infrastructure also hosts Find/Checksums/File
  List. Working tools keep taskbar eligibility; ordinary dialogs do not.

## Verification

- Viewer/comparator regressions: 18 tests passed across file choice, fonts,
  keyboard input, UI geometry and wrapping. After a final toolbar-gap correction,
  all 11 affected viewer layout/wrap tests passed again. The PDF 320×200 case
  already passed, so its layout was retained.
- Real Windows native probe passed against the final test artifact at 225%
  scaling: actual HWND owner/style,
  decorated work-area bounds, movement beyond the main window,
  minimize/activate, deferred creation cancellation and working-tool taskbar
  eligibility. This establishes native behavior, not Explorer taskbar grouping.
- Offline Linux library check passed for the native foundation using the
  existing Linux target. Linux desktop runtime was not tested in this pass.
- The final Windows UI batch passed **982 tests**: 802 library tests and 180
  tests across 35 integration targets (`polish-tests.log`). The desktop-only
  native probe, ignored in that headless batch, passed separately. Together with
  the 18 viewer/comparator tests, this is 1,000 automated tests plus the native
  desktop probe; the repeated 11 viewer checks are not counted twice.
- Regressions cover actual clipped bounds, keyboard/pointer input, resize with
  unsaved drafts, selected-row visibility, small-window dismissal and primary
  action focus after scrolling. The empty-tooltip regression was first
  reproduced with actual pointer hover and the tooltip delay; the shared fix
  now passes alongside the existing nonempty-tooltip placement check.
- Rendered captures were inspected for settings, operation forms, metadata,
  Find, Jobs, Checksums, File List, Hot Paths, notices, viewers and comparison.
  Checks include minimum 320×200 geometry, mouse/touch density, large fonts and
  light/dark themes. Logs and captures are under `_build/qa/native-dialogs/`.
- Scoped formatting and whitespace checks passed. The source-size check passed
  for all 1,775 handwritten source files (at most 100,000 bytes each).
- Strict Clippy passed with `-D warnings` for the UI, viewer UI, comparator UI
  libraries and the `me` application target, using the locked offline Windows
  toolchain. Logs: `clippy-ui.log` and `clippy-app.log`.
- The debug application build passed (`app-build.log`). The private installation
  was updated with SHA-256 readback verification and matching plugin-runtime
  registration. Its actual `--ui-smoke-test --software` run completed with exit
  code 0 and captured `app-smoke.png`: both native filesystem panels populated,
  the toolbar and bottom row rendered, and no startup error appeared. The smoke
  run used isolated in-memory preferences rather than the user's saved settings.

## Limits

The main application minimum remains 320×200. At that extreme, secondary content
and actions scroll; the forms do not reduce configured font sizes to hide an
overflow. Viewers retain their pre-existing 12px compact-toolbar cap and initially
cover the main window. Document font choices are unchanged.

This work does not claim completion of the remaining native-window migration:
operation/settings/conflict forms still use their existing embedded hosts until
their separate migration steps are implemented. No backend authority, filesystem
operation policy, dependency or plugin API was changed by the layout polish.
