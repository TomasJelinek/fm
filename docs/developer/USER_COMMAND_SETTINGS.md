# Custom Commands settings

Options → Custom Commands edits the ordered configuration stored by schema 17.
New creates a stable ID; changing the name, enabled state, label, icon or order
does not change that identity. Delete removes that command's shortcut overrides
through `UserPreferences::replace_user_commands`, preserving unrelated bindings.
The toolbar's Settings action selects the corresponding ID on this page.

Drag a list row to change command order; the insertion line shows its destination.
Dropping stages the new order within Options and keeps the dragged command
selected. OK persists it; Cancel restores the original order. Up/Down remain available for keyboard
and accessibility use. The list scrolls at its edges while dragging, and keyboard
navigation reveals the selected row. Dirty drafts require Update command or Discard before
reordering. Escape, right-click, pointer cancellation, closing Options, stale
configuration and releases outside the actually visible list cancel the drag.

The gesture captures the command ID and immutable configuration revision, rather
than retaining a row index. Drop validates that capture before calling the existing
list transaction. Its visible bounds intersect the list and Options content scroll
viewport, including small-window and ancestor clipping, and exclude overlay
scrollbar controls. The interaction follows
Salamander's `src/edtlbwnd.cpp::MoveItem` and mouse/timer/cancel handling, with User
Menu's `EDTLBN_MOVEITEM2` integration in `src/dialogs4.cpp`; no reference source or
assets are copied.

The page follows Salamander's User Menu: Menu Contents and its New/Delete/Up/Down
controls appear above the selected item's Command, Arguments and Initial
directory fields. The reference layout is `src/lang/lang.rc:247`; field-specific
menus and local editing are in `src/dialogs4.cpp:1980–2341` and
`src/execute.cpp:223/321/407/1965`. No reference assets or source are copied.

Program mode has an absolute native executable path and editable argument rows.
Each row is one argument, including empty rows; spaces and quotes are literal,
and no shell splitting is applied. Add/Remove and Up/Down edit those boundaries.
The argument value menu inserts captured file/folder values as complete arguments.
`${selected.paths}` is a whole Program argument in Single mode; the core rejects
invalid combinations. Terminal script is an explicit advanced choice with a shell
and required captured context declarations. Its script runs literally.

Update command validates and stages one command, allowing another command to be
edited. The common OK footer also validates any remaining draft, saves the complete
Options transaction, and closes only after storage succeeds. A failed save keeps
the dialog open for retry. Discard changes reloads the currently staged command;
Cancel discards the entire Options session, including applied commands and order.
Changing the selected command while a draft is dirty requires Update command or
Discard. A configuration replacement invalidates stale
draft callbacks; no displayed path is used as an existing native identity.

Executable and icon fields remember whether the user edited that path. Renaming
another field preserves the original native bytes or UTF-16, including paths that
cannot be represented as ordinary Unicode. A path edit creates a new native
literal only when it is unambiguous. Editing within a lossy parent while the
entered path still contains U+FFFD is refused; it never silently rewrites that
native parent. Keep the original path or enter a fully representable replacement.
Images are requested by the toolbar's
backend image service; this editor performs no filesystem or image loading.

Browse buttons use the backend's native program, directory and image pickers.
Requests capture the target field, draft revision and configuration identity;
typing, changing pages or closing Options invalidates the result. Cancel leaves
the draft unchanged. Program and image picks retain the original native path,
including values whose displayed Unicode caption is lossy. Initial directory is
currently a template string: unrepresentable native directory picks are refused,
and dollar signs in a picked literal are escaped for that template. The Active
folder button remains available without converting a native folder to text.

Options → Editors shares the argument-row control and a Browse button. Its draft
also survives page switches; Cancel discards it. Use the system default editor
or choose a program and include a complete File argument for F4. OK validates
the editor configuration once; editing or browsing never launches the editor.

Preview validates an immutable transient configuration and invokes the backend's
Preview command only. It neither applies settings nor executes processes. Once
accepted, Jobs displays the captured preparation result while Options remains
open with its draft. Disabled commands can be previewed;
execution remains disabled. Availability and native-shell errors are backend
results. See [the backend contract](USER_COMMANDS.md) for provider and shell limits.

Keyboard settings uses `ShortcutMap::with_user_commands` for the catalog,
assignment checks, help and routing hints. Its immutable cache tracks both shortcut
overrides and command configuration identity. Rename/reorder preserves the selected
action by stable ID; deleting that action clears its pending candidate, so a late
Assign callback cannot bind a different action. Dynamic commands have no default
keys and are scoped to the file list outside filename search.

The page uses the existing Options scrolling body and fixed close footer. Its
validation debounce does not replace live text fields, preventing cursor resets
while editing. Command count, argument count and aggregate configuration limits
come from the core; malformed or oversized inputs are rejected with visible
validation instead of truncated.

Focused checks live in `presenter/user_command_settings_tests.rs`, the pure form
tests and `tests/user_command_settings_ui.rs`. Qualification evidence is recorded
in [the settings report](../verification/2026-11-07-custom-command-settings.md);
raw test output remains under `_build/goal/commands/`.
The earlier settings target passed 12 software-window cases on Windows and Linux;
that evidence predates the argument-row/native-picker refactor. Current refactor
qualification is recorded separately under `_build/goal/commands-refactor/`.
See [the refactor verification](../verification/2026-11-07-user-menu-refactor.md)
for the fresh 510 Windows / 506 Linux functional scope and remaining strict gate.
The checks exercise pointer, keyboard and widget accessibility actions;
real-desktop acceptance is qualified separately.
