# Find form and filter-help ownership

Find's basic query is reformatted into one aligned grid: a visible File name
label with the text field and compact Masks/Regex/Expression selector; Search
folders with picker and Remove; and Containing text with its input and case
option. Name case and recursion stay beside their associated fields. Narrow
windows stack the same controls; advanced widgets remain instantiated when
collapsed, preserving the draft. The query area uses its preferred height at
normal sizes, capped to leave results space. Compact running searches still give
the disabled query area to results, and the minimum-size idle editor reserves
space for its label and text input. Existing native ToolWindow theme/fonts and
footer action roles are retained.

The staged candidate was stored under
`_build/qa/dialog-polish-20261111/find-form-candidate/`, then applied only after
the coordinator released the generated-binding freeze. Original source snapshots
were compared before replacement. No search backend, query grammar, callback
contract, lazy-result access, plugin API or dependency was changed.

Panel Filter help previously opened through MainWindow's modeless information
manager while MainWindow was disabled by the modal filter. It now has one native
HelpWindow owned modally by that exact filter occurrence. Dismissing Help returns
to the retained filter draft; retiring the filter closes its help first. Parent
and child session checks reject stale callbacks. Ordinary keyboard Help remains
under the existing information-window manager. Help creation failure is shown in
the active filter's existing error field.

## Verification status

New Find geometry coverage checks basic-control visibility, aligned field starts,
compact mode width and useful result height at 780×560 desktop, 900×700 touch and
1100×800 with larger text. Existing `find_interactions` tests cover advanced-state
retention, invalid intervals, actual pointer/keyboard input, Stop/restart, lazy
results and 320×200 at 32px. `find_windows` retains all-theme/density and independent
window lifetime coverage.

Filter-help tests exercise nested modal leases, exact parent/child sessions,
stale Help dismissal, child-first parent retirement and unchanged filter action
authority. Review also found that Linux cannot disable native owners directly:
the filter now registers its own Slint key capture and pointer/scroll gate with
the modal owner. `native_filter_help_blocks_owner_pointer_and_keyboard_until_help_closes`
uses the software adapter to verify actual edits, Enter/Escape and Show all/OK/Cancel
clicks are ignored under Help, then normal editing/submission resumes on close.

The first `dialog_theme` run (`_build/qa/dialog-polish-20261111/ui-dialog_theme.log`)
exposed a separate disabled-action issue: the focused stock Button could emit
Enter after disabling. The shared application ActionButton now checks `enabled`
inside its activation function, before toggle, focus or callback dispatch. The
same test asserts accessibility and keyboard activation separately; its Tab
expectation follows the new Subfolders → Containing text query order.
The coordinator's v2 targeted run passed:

- `find_interactions`: 7/7, including the new form geometry and minimum-size
  pointer/keyboard behavior (`_build/qa/dialog-polish-20261111/ui-v2-find_interactions.log`).
- `dialog_theme`: 4/4, including disabled Stop's keyboard/accessibility guard
  (`_build/qa/dialog-polish-20261111/ui-v2-dialog_theme.log`).
- Filter lifetime unit tests: 2/2
  (`_build/qa/dialog-polish-20261111/unit-v2-panel_filter_windows-tests.log`).
- `native_filter_hotpath`: 11/11, including actual blocked owner pointer/key input
  and resumed editing after Help closes
  (`_build/qa/dialog-polish-20261111/ui-v2-native_filter_hotpath.log`).

The three current form renders were visually inspected under
`_build/qa/dialog-polish-20261111/ui-renders-v2/`:
`find-form-0-0-13-780x560.bmp`, `find-form-1-1-17-900x700.bmp` and
`find-form-3-0-22-1100x800.bmp`. Labels and field starts align, the compact mode
selector leaves the name field room, basic controls remain readable, and results
retain substantial space. These are software-renderer screenshots, not an
interactive native desktop verification.

`find_windows` passed 5/6; its all-theme/density action test clicked Details at a
coordinate underneath fixed Close when the 320px touch footer overflowed
(`_build/qa/dialog-polish-20261111/ui-v2-find_windows.log`, "Missing Copy"). The
fixture now scrolls each real action wholly into its Flickable viewport before
clicking, and asserts Details/Query state transitions. The same complete theme,
density and size matrix remains. The corrected v3 `find_windows` run passed all
6 tests, including that full theme/density/size matrix in 16.16 seconds
(`_build/qa/dialog-polish-20261111/v3-find_windows.log`). Source formatting and
whitespace checks passed. All targeted checks for this Find/Help change now pass;
interactive native desktop qualification remains separate from these software
window and render results.