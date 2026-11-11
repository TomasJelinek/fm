# UI and operations follow-up, 11 November 2026

This is the active follow-up queue. It records the user's latest corrections;
older plans do not override these decisions. Keep targeted checks during work,
then run the combined affected checks after the batch is implemented.

## Window policy (supersedes the earlier all-native-dialog migration)

Only modeless working windows should be freely movable outside the main window.
Attributes / Times / Case stays embedded; its native migration was canceled before
any edits. Ordinary confirmations and settings are modal and have no taskbar item.
Make File List (Ctrl+M) is modal, has an action icon/title, and its preview expands
with the window. Find and other independent modeless tools remain discoverable.
Existing native modal hosts need a policy review; do not convert more embedded
modal forms merely to satisfy the superseded migration plan.

## Active implementation

- [x] Inset split-button hover surfaces for Hot Paths, terminal and history. Eight
  focused tests and the debug build pass; committed as `5ca372b`.
- [x] Remove persistent pointer-focus outlines from action toolbar buttons while
  retaining keyboard focus feedback and real checked/active states.
- [x] Panel Filter: align the checkbox, compact the form/footer, and add accessible
  help for Masks, Regex and Expression with actual supported syntax/examples.
- [x] Rename / New empty file: place location on its own line, remove irrelevant
  source date/size from Rename, and size the form for its content.
- [x] Rename collision: reuse the Copy/Move overwrite prompt and safe, consented
  replacement; preserve same-object protection and no-follow identity checks.
- [x] Unify application fonts, themed action buttons and meaningful action icons
  across dialogs; fix the first refresh of Find and polish its layout.
- [x] Enable native checksum capture from acknowledged inline-tree selections.
- [x] Qualify the bounded DPI-scaled viewer refill candidate against the supplied
  JSON. Three engine regressions pass; two native JSON pairs reduce the maximum content gap (117/93 ms to 79/78 ms), with similar typical latency.

## Subsequent assigned work

- [x] Ctrl+M: modal ownership/no taskbar entry, clear icon/title and clearer form;
  expand preview to fill remaining space as the window grows.
- [x] Count directory junction/link targets. Detect cycles using stable filesystem
  identity along the traversal ancestry, not a path-length or depth guess. Preserve
  cancellation and report inaccessible/cyclic branches without infinite recursion.
- [x] Attributes: explain/remove unnecessary Refresh; locale-formatted date/time
  editing; equal compact heights for date/time fields and adjacent controls.
- [x] Jobs: Clear all finished, or Clear finished without errors. The latter also
  clears error-free cancelled receipts, while retaining failures, recorded errors
  and unresolved recovery/durability issues. Both retain all unfinished jobs.
- [x] Probe subfolder availability before the first disclosure click, asynchronously.
- [x] Canceling Copy/Move/Delete confirmation must not open Jobs or leave a job entry.
- [x] Drag/drop: reliable cursor feedback with Copy/Move text, unavailable indication
  for self-drop, and Escape cancellation.
- [x] Rectangle selection: update details/selected size during dragging, not only on
  release, with backend identity/generation guards preserved.
- [x] Options: one Refresh/Rescan all plugins action shared across the three kinds.
- [x] Replace the parent-row up arrow with a more restrained upward chevron.
- [ ] Diagnose and fix the actual failing GitHub workflow, then verify the rerun.

Source requirements for platform separation, backend-only non-UI work, Unicode,
lossless native paths, bounded async ownership and documented free dependencies
remain unchanged. Checked items have focused evidence in the linked verification reports. The remaining CI checkbox requires a green rerun; source fixes alone do not close it.
## Verification handoff

All 124 Windows UI integration targets now have passing evidence across the
completed sweep and focused reruns. The original manifest recorded 115 successful
and nine failed targets, including 39 successful results retained when the
unchanged-build sweep restarted
(`_build/qa/dialog-polish-20261111/full-ui-results.json`). The first
follow-ups resolved eight failures: `final-regressions-results.json` recorded
four of five targets passing and `final-tree-results.json` all five tree targets.
The last `file_actions_ui` focus-handoff failure reproduced a real Presenter
borrow panic during drag focus loss. After its correction,
`gesture-final-results.json` passed all five targets, including file actions 9/9,
drag integration 11/11 and rectangle selection 12/12. This is aggregate evidence
from the sweep and reruns, not one monolithic final run.

The desktop default is 12, saved sizes including 11 remain preserved, and touch
stays 15. Settings tests passed 16/16; `fonts` and `options` UI targets passed.
Native HWND checks verify the shared modal host policy; Linux native-plugin and
platform listing/mutation tests pass separately. See:

- [Fonts, metadata and drag feedback](../../verification/2026-11-11-dialog-fonts-metadata-and-drag-feedback.md)
- [Folder hints, junctions and cancellation](../../verification/2026-11-11-folder-presence-sizing.md)
- [Find and filter help](../../verification/2026-11-11-find-form-and-filter-help.md)
- [Rename overwrite safety](../../verification/2026-11-11-rename-overwrite.md)
- [Jobs history](../../verification/2026-11-11-jobs-history.md)
- [Viewer opening and measured refill](../../verification/2026-11-11-text-opening-scroll.md)

The CI prerequisites fixes are committed as `0dfd523` and `cf3c938`; the footer
regression correction is `d0f27a0`. That CI run completed with failures: Linux
ColorPicker minimum-size Cancel geometry and Windows fallback-menu focus handoff
(`github-d0-130680114769.log` and `github-d0-windows.log` under the same evidence
directory). Local corrections and targeted passes do not close the unchecked CI
item; a green rerun is still required. Final unit qualification, the application
build and deployment must be recorded separately before claiming that the new
executable contains this batch.

Final debug build and native/software startup qualification passed; the matching
private runtime is registered. See the final build section of the fonts/metadata
verification report for the executable hash and exact artifacts. The current
GitHub rerun must still pass before closing the CI item.
