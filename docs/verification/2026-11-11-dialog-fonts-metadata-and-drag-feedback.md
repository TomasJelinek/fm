# Dialog appearance, metadata editing and drag feedback — 11 November 2026

This records the source changes for the [active follow-up queue](../superpowers/plans/2026-11-11-dialog-polish-followup.md).
All 124 Windows UI integration targets have passed across the completed sweep and
subsequent focused reruns. The original sweep recorded nine failing targets;
the corrections and their rerun evidence are recorded below. This was not one
monolithic final run; CI remains separate from the local build qualification below.
No dependency or native metadata-window migration was added.

## Appearance and Make File List

- The desktop font default is now 12 logical pixels; touch remains 15. Existing
  saved font sizes, including 11, stay unchanged. First-run preferences, older
  settings without font fields and Reset to defaults use the new desktop default.
  Default-only Options/migration assertions were updated; the existing persistence
  regression now explicitly retains a saved desktop size of 11. The Windows
  settings suite passed 16/16 in `font12-settings.log`, including legacy defaults,
  saved-size preservation and font validation. The subsequent UI `fonts` and
  `options` targets passed 1/1 each (`full-ui-fonts.log`, `full-ui-options.log`),
  covering independent font settings and the revised first-run default. Earlier
  screenshots retain their recorded font sizes.
- Application window content uses the selected theme font. An empty font setting
  resolves to the existing Segoe UI fallback in MainWindow, ToolWindow, Viewer
  and Comparator. Native title bars retain the operating system's font.
- Main appearance preferences publish before Find and the other adapters copy
  them, removing Find's first-refresh theme/font lag.
- Shared action buttons paint neutral, primary and destructive roles from Theme,
  with compact meaningful icons. Native button keyboard/focus semantics remain
  behind the themed presentation. Shared wrapping checkboxes center the check
  glyph and focus frame in the fixed indicator slot.
- Dialog footers reserve horizontal scrollbar space only when their actions
  overflow. Notice Copy details and Close use their matching action icons.
- Ctrl+M Make File List uses the existing owner-modal DialogHost and ordinary
  dialog role, which does not create a separate taskbar entry. The title/action
  icon remain visible; the read-only preview expands with the body. Resizing
  preserves the edited template and selection. Cancel/backpressure and the
  system save picker's owner retain the existing session contract.

This is a scoped appearance change: it does not replace every stock Slint
control or change Viewer/Comparator document fonts. The existing Comparator
file-choice primary button is outside the shared action-button conversion.

## Embedded Attributes / Times / Case

The dialog remains embedded, following the user's correction. Refresh duplicated
Review changes and was removed; Review changes remains the preview action before
Apply. Timestamp edits use a single control-height line beside their action.
Visual inspection found wrapped Times help overlapping its status at font 32;
the scroll body now retains intrinsic height instead of compressing that text.

Metadata preview workers format current/proposed timestamps through LocaleService
and capture the platform's numeric date/time input pattern. The UI parses that
bounded captured pattern without calling operating-system or filesystem services.
Four-digit years and explicit `Z` or `+/-HH:MM` offsets keep the instant unambiguous;
there is no implicit local/UTC conversion or DST guess. ISO 8601 input remains
supported. Native patterns that cannot be represented safely, including named-month
or non-Gregorian input patterns, retain ISO entry while display stays localized.
Invalid dates, missing offsets and leap-second input are rejected.

Existing MetadataJobId/version ownership, preview/apply staging, cancellation,
and stale-receipt guards remain authoritative.

## Internal drag/drop feedback

The internal pointer badge remains visible while a source is preparing, while its
destination is being checked, and for Copy, Move or Not allowed outcomes. The
cursor follows the same state. External native drag handoff retains the existing
OLE/XDND feedback authority.

A typed drag/revision request asks the backend for an advisory hover result.
The UI cancels its token immediately on destination/modifier changes, leave,
release, Escape or native handoff. Only the matching active revision can update
feedback. Tree hover captures have independent request identities and cannot
consume a final drop's captured destination. Retired capabilities go through the
existing backend cleanup path.

The backend worker coalesces obsolete requests, checks source/destination identity,
ancestry and actual native device information, and never infers volume identity
from drive letters. Hover never admits a transfer. Release independently validates
its destination, including when preview is still pending. Thus self/descendant
rejection does not depend on a completed hover check.

A related review fixed live rectangle publication: copy the bounded status rows
and row structure, release presenter borrows, then notify Slint models. The bridge
also rejects or cancels selection gestures while its owner is modal-blocked.

## Recorded and prepared verification

The shared Windows run produced these logs under
`_build/qa/dialog-polish-20261111/`:

| Target | First focused result |
| --- | --- |
| `ui-drag_drop.log` | 2 passed |
| `ui-file_list_window.log` | 6 passed |
| `ui-metadata_editor.log` | 5 passed, before the large-font overlap correction |
| `ui-metadata_review.log` | 6 passed |
| `ui-dialog_theme.log` | 3 passed, 1 failed |

The first theme failure was Find Stop still dispatching while already stopping.
The second shared Windows run passed `ui-v2-dialog_theme.log` (4/4),
`ui-v2-metadata_editor.log` (5/5, including the no-overlap matrix), and
`ui-v2-selection_rectangle_ui.log` (11/11, including model-notification reentry
and modal-gate cancellation). Reviewed `ui-renders-v2` Times at font 32 in both
densities now scrolls its long text without overlapping status/footer. Notice
520×300 shows the selected font and compact aligned actions. File List at
320×200/390×300 retains visible Cancel; oversized neighboring actions use the
footer's horizontal scroll.

Final focused Windows evidence adds:

| Log | Result and exercised boundary |
| --- | --- |
| `native-owner-final.log` | 1/1 passed: `native_owned_dialog_has_no_separate_taskbar_button` on an interactive Windows desktop |
| `file-list-final.log` | 14/14 presenter tests passed, including pending cancellation when the command queue is full and stale generation/session rejection |
| `v3-windows-locale.log` | 6/6 platform locale tests passed, including native user overrides, historical timezone conversion and stable four-digit-year input patterns |
| `metadata-edit-final.log` | 11/11 native metadata-edit tests passed, including case/time combinations, collision/cancellation, compression outcomes and raw UTF-16/long paths |

The native-host test realized actual HWNDs for a generic Help dialog and Find
working window. It checked the modal child's owner, visible state and ordinary
dialog styles without `WS_EX_APPWINDOW`, while the main window retained its
application-window style. The independent working window had no native owner,
was visible and retained the taskbar/Alt-Tab-eligible application-window style.
This is shared-host evidence; it does not claim a separate fresh File List
native-window probe or a desktop OLE/XDND drag gesture.

The focused coverage comprises:

- `dialog_theme`: first-refresh Find appearance, content font propagation,
  primary/neutral colors, compact icon alignment, checkbox/button pointer and
  keyboard behavior, disabled actions and compact Notice footer.
- `file_list_window`: modal lease lifetime, queue refusal, resizable read-only
  preview, Unicode template selection retention and compact Cancel access.
- Metadata UI/parser/presentation, platform locale-pattern and engine metadata
  tests: localized examples, stable explicit-offset parsing, compact fields,
  typed preview handoff and formatting off the UI thread.
- Presenter drag/tree and installed `drag_drop`: immediate feedback, current-only
  revisions, distinct tree capture, Escape cancellation, no hover admission and
  no release after cancellation. Backend preview tests are owned by the backend
  implementer and cover coalescing, device/modifier choice and release races.
- `selection_rectangle_ui`: installed row/details model notifications can borrow
  the presenter mutably, and modal owner gating cancels without committing.

Focused rustfmt and whitespace checks passed for the authored Rust changes.
The combined Windows integration compile passed. The completed
`full-ui-results.json` manifest records 124 targets: 115 successful and nine
failed, with 39 successful results retained when the same unchanged-build sweep
was restarted; those targets were not redundantly rerun. The failed targets were `copy_errors`, `drag_integration`,
`file_actions_ui`, `native_operation_windows`, `tree_bulk_selection`, `tree_copy`,
`tree_folder_sizes`, `tree_prefix_search` and `tree_viewers`.

The subsequent targeted evidence is:

| Artifact | Result |
| --- | --- |
| `final-regressions-results.json` | 4/5 targets passed: `command_menus` 1/1, `copy_errors` 9/9, `drag_integration` 9/9 and `native_operation_windows` 9/9; `file_actions_ui` remained 8/9 |
| `final-tree-results.json` | 5/5 targets passed: bulk selection 8/8, copy 2/2, folder sizes 7/7, prefix search 5/5 and viewers 2/2 |
| `gesture-final-results.json` | 5/5 targets passed: drag/drop 2/2, drag integration 11/11, drag ordering 3/3, file actions 9/9 and rectangle selection 12/12 |
| `full-ui-color_picker_popup.log` | 3/3 passed with the responsive full-caption footer at the minimum size |
| `full-ui-tool_reflow.log` | 3/3 passed, including the compact Find mask editor and visible empty-state line at font 32 |
| `full-ui-user_command_settings_ui.log` | 14/14 passed, including current-draft validation after batched accessibility reorder/edit |

The first two targeted result sets resolved eight of the original nine failing
targets. The remaining `file_actions_ui` case,
`copy_progress_can_take_focus_from_a_fallback_menu_without_reentrant_borrow`,
reproduced a real `RefCell already borrowed` panic in `drag_bridge.rs:330` during
focus loss (`final-regressions-file_actions_ui.log`). The deferred menu dismissal
and queued-action rejection cases already passed in that run. After the drag
focus-loss correction, `gesture-final-file_actions_ui.log` passed all 9 tests,
including that regression. The four related gesture/rectangle targets also
passed in `gesture-final-results.json`; `gesture-final-build.log` records their
successful compilation. All 124 original integration targets therefore have
passing evidence across the sweep and focused reruns, not one monolithic final
run. The final focused UI unit run also passed: lifecycle 5/5, drag/drop 10/10,
tree drag 8/8 and rectangle selection 5/5 (`gesture-unit-*.log`). Application
qualification is recorded below; a green CI rerun remains separate.

GitHub CI for `d0f27a0` completed with failures, not an in-progress result. Linux
failed the minimum-size ColorPicker Cancel geometry
(`github-d0-130680114769.log`); Windows failed the fallback-menu focus-handoff
case (`github-d0-windows.log`). The local fixes above do not constitute a green
CI rerun. No new desktop OLE/XDND gesture or Linux desktop claim is made by this note.


## Linux directory-link listing follow-up

Linux native listings previously reported directory symlinks as generic Symlink,
so explicit folder-size root admission ignored them. The native plugin and local
Linux adapter now optionally probe symlink target directory kind using
`O_PATH | O_DIRECTORY`; no target contents or full target metadata are read.
File, broken, looping and inaccessible targets retain Symlink. The original link
path, raw name and no-follow Unix metadata remain unchanged. Plugin Stat repeats
the same hint so SDK kind revalidation accepts a listed DirectoryLink.

New plugin/platform tests cover actual listing kinds, raw non-UTF-8 names,
retained symlink mode, file/broken/loop links, cancellation, content-read rejection
and deleting only the link through a separate no-follow mutation capture. The
engine's selected-link sizing fixture now takes its kind from actual enumeration.
WSL Ubuntu 24.04 plugin verification passed: `directory_links` 2/2 and `native`
5/5 (`linux-native-classification.log`). The raw-byte fixtures use the existing
`MIDNIGHT_NATIVE_FIXTURE_ROOT` override under `/tmp/fm-interaction/_build/`;
DrvFS cannot preserve arbitrary Unix byte names. The platform `directory_links`
target also passed 2/2 (`linux-platform-classification.log`), including actual
listing-to-mutation identity separation and cancellation.

The subsequent Linux qualification passed strict Clippy with `--all-targets --
-D warnings` for both the native plugin and platform crate
(`linux-native-classification-clippy.log` and
`linux-platform-classification-clippy.log`). No lint fixes were required.
The filtered engine `folder_sizes::native_tests` passed 3/3
(`linux-engine-native-link-sizes.log`): actual enumerated DirectoryLink selection,
self/ancestor cycle stopping, unavailable-target partial results and cancellation.
Its cached build completed in 1m45s. Formatting and scoped whitespace checks passed.
## Final Windows build qualification

- Independent review of exact-ID cancellation, backend queue retries, native
  handoff ordering and rectangle borrow handling found no actionable issue.
- `cargo fmt --all -- --check`, the 100,000-byte source limit (1,935 files), and
  strict Clippy for the application/UI/viewer packages with `--all-targets` pass.
  The unchanged vendored femtovg dependency still emits two nonfatal warnings.
- The final debug build completed in 26.11 seconds (`final-debug-build.log`).
  `D:/experiments/dev/FM/_build/debug/me.exe` has SHA-256
  `fed5b6aa80f45b48a5e5efbd2e7c35878a6d918541af7b6235c97dd9edfa8267`.
- The matching private plugin runtime is registered at
  `C:/Users/tjelinek/MidnightExplorer-20261111-dialogs-fed5b6aa`.
  Windows plugin binaries were reused unchanged; no Windows plugin/API source
  changed in this batch. Linux native-plugin changes have separate evidence above.
- The actual `_build/debug/me.exe` passed backend listing, native-renderer UI and
  software-renderer UI smoke runs: all exited zero with empty stderr. The two UI
  snapshots show both populated panels, consistent fonts, intact toolbars and
  the parent chevron. Saved user preferences retained the same SHA-256.
- Artifacts are in `_build/qa/dialog-polish-20261111/`: `final-clippy-v3.log`,
  `final-format-v3.log`, `source-size-v3.log`, `final-deploy.log`,
  `final-smoke-{backend,native,software}.log`, and `final-{native,software}.png`.
  These build/test artifacts are not committed.
