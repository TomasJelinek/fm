# Small main window

The native minimum is exactly **320×200 logical pixels** in mouse and touch
density, including larger fonts. The preferred size remains **1340×820**.

Below 640 pixels wide, 360 pixels high, or the space needed for the configured
controls, the active pane occupies the available width. The rail activates the
other pane. Path, file list, command editor and footer remain present; toolbars,
tabs, column headings and detailed status return when the window grows. Compact
file rows show names in one column without changing the saved view preference.
Compact path and command text cap at 24 and 28 pixels respectively so the
largest configured font remains usable within the fixed minimum.
Errors retain their complete text in a tooltip. The command directory retains
its complete text in the existing horizontal viewport.

Function keys pan horizontally by horizontal wheel/trackpad or drag when they cannot fit.
Options and Jobs stay fixed at the right edge. A held Ctrl changes every slot to
the effective configured action; unassigned slots show their key, stay disabled
and explain how to assign a command. Narrow keys show readable key captions and
retain complete action tooltips. Ctrl prefixes are omitted from mapped visual
captions while Ctrl is held; accessibility and tooltips retain the full chord.
Mouse presses retain their original action and logical pane even if Ctrl or the
active pane changes before release. The native event observer captures this
before Slint's scrolling container forwards a delayed press; dragging still pans
the footer without executing a command.

The footer consumes presenter capabilities: Rename/View/Edit require a real
focused entry, while Copy/Delete accept a real selection or focused entry.
Parent navigation does not become an operation target. Create-directory and
navigation remain independent of the current file target.
The Attributes toolbar uses the same selection capability and its dedicated
vector icon. Archive panes retain F5 copy-out when a real source is available.

`midnight-ui/tests/small_window.rs` checks native layout constraints, actual
control and row geometry at 320×200 in both densities and 11/15/24/32-pixel fonts,
activation of the other pane, restoration to 1340×820, 640/800-pixel toolbars,
splitter movement, long Unicode command paths, dialog actions and all twelve
normal/Ctrl captions after shortcut remapping. Rendered evidence belongs in
`_build/small-window/`.
`commander_edges.rs` exercises immediate press/release routing at both preferred
and minimum sizes, including modifier changes, pane switches, disabled slots
and drag cancellation.

Verified on Windows on 2026-11-07 with the locked offline dependencies:
`commander_edges` (9), `small_window` (4), `compact_feedback` (1),
`dialog_layout` (1), and `panel_refinements` (1): **16 passed**. The final
320×200 mouse/font11 and touch/font32 images and both Ctrl footer widths were
visually reviewed. The final dialog batch passed 32 tests, including the four
small-window tests, in `dialog_drag`, `commander_dialogs`, `keyboard_options`,
`metadata_editor`, `dialog_layout`, and `small_window`; its log is
`_build/qa/dialogs-final-verified.log`.

Rubber-band selection remains pending. The existing native row drag/drop and
Ctrl/Shift selection are unchanged. Correct rectangular selection across Brief
and Thumbnails requires a disjoint-range backend contract, archive support and
stale-list validation; this change does not introduce a partial single-view mode.
