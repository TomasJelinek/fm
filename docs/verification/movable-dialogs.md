# Movable dialogs

Dialogs retain their existing popup or modal lifetime. Their shared header moves
the inner body within the current application viewport, and shrinking the window
clamps that body back into view. Reopening starts centered. Menus and tooltips
do not use draggable frames.

Coverage includes Options, Jobs, Help, About, Hot Paths, directory comparison,
panel filters, notices, copy/metadata conflicts, the attributes editor, rename,
Trash/permanent-delete confirmation, directory/file creation and copy drafts.
The attributes editor uses the original `icons/attributes.svg` document-and-clock
icon and omits the redundant captured-file list; validation and guarded Apply
remain unchanged.

`midnight-ui/tests/dialog_drag.rs` sends real pointer press/move/release events,
checks two consecutive moves, clamps extreme movement, resizes open dialogs to
320×200, checks close controls at the largest touch font, exercises Escape and
outside-click policy, checks centering on reopen, and types into the retained
name draft after dragging. The initial test failed before implementation because
the Options header did not move. The shared implementation uses an inner body
because the pinned Slint embedded-popup position tracker does not subscribe to
x/y-only changes.

`small_window`, `dialog_layout` and `metadata_editor` provide existing geometry,
action and guarded-apply regressions. Rendered evidence is kept under `_build/`.
The metadata test also checks the actual ancestor Flickable bounds: before the
final row-height fix a 32-pixel Fluent button was clipped by a 25-pixel viewport.
Scrollable action rows now honor both the button's preferred height and Fluent's
32-pixel minimum. The narrow layouts retain their scrollbar allowance.

`small_actions` covers long Trash, permanent-delete and directory-creation
confirmation labels at 320×200 with the largest touch font. It verifies horizontal
scrolling without confirmation, a pinned Cancel button, and a usable confirmation
viewport. It also checks F9/F10 menu scrolling, scrollbar-click persistence,
selection and Escape, plus fallback file-menu bounds, arrow-key reveal/Return
and pointer selection. Before the fix all three tests failed: the first two
surfaces had no scroll viewport and the fallback menu was 288 pixels tall in a
200-pixel window (`_build/qa/small-actions-red.log`).

Run the focused suite with the normal project wrapper:

```powershell
./scripts/cargo.ps1 test -p midnight-ui --test dialog_drag --test small_window `
    --test dialog_layout --test metadata_editor --test commander_dialogs `
    --test keyboard_options --locked --offline -- --test-threads=1
```

Windows verification on 2026-11-07 completed with exit 0: **32 tests passed**
(5 drag/focus, 2 comparison/filter, 1 full dialog-layout matrix, 19 Options
keyboard, 1 metadata-editor matrix and 4 small-window tests). The final log is
`_build/qa/dialogs-final-verified.log`; the deliberate clipping failure is in
`_build/qa/dialog-footer-clip-red.log`. Final screenshots in
`_build/qa/dialogs/` were inspected at normal size and 320×200, including the
32-pixel touch layout.

The compact-action follow-up also passed on Windows: **3 new tests** in
`_build/qa/small-actions-verified.log`, plus **15 existing tests** across
`command_menus`, `commander_dialogs`, `commander_tools`, `delete_confirmation`,
`dialog_drag` and `mutation_controls` on the same bindings. Their results are in
`_build/qa/small-actions-fixed.log`; that earlier run still records a test-only
assumption corrected in the final run (an embedded dialog retained the previous
horizontal scroll offset). The regression now verifies both scroll directions.
Screenshots under `_build/qa/compact-actions/` were inspected at 320×200.
