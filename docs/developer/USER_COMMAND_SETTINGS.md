# Custom Commands settings

Options → Custom Commands edits the ordered configuration stored by schema 17.
New creates a stable ID; changing the name, enabled state, label, icon or order
does not change that identity. Delete removes that command's shortcut overrides
through `UserPreferences::replace_user_commands`, preserving unrelated bindings.
The toolbar's Settings action selects the corresponding ID on this page.

Drag a list row to change command order; the insertion line shows its destination.
Dropping commits one preferences transaction and keeps the dragged command
selected. A same-position drop does not save. Up/Down remain available for keyboard
and accessibility use. The list scrolls at its edges while dragging, and keyboard
navigation reveals the selected row. Dirty drafts require Apply or Cancel before
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

Program mode has an absolute native executable path and a JSON list of argument
templates. Each JSON string is one argument, including empty strings; no shell
splitting is applied. Terminal mode has literal script text, a shell choice and
explicit required scalar context declarations. The variable picker adds a quoted
environment reference and its declaration together. `${selected.paths}` is a whole
Program argument in Single mode; the core rejects invalid combinations.

Apply validates the complete candidate configuration through the core before
using the ordinary preferences transaction. A refused save retains explicitly
unsaved local preferences and the existing Save again action. Cancel reloads the
current saved-in-memory configuration. Changing the selected command while a draft
is dirty requires Apply or Cancel. A configuration replacement invalidates stale
draft callbacks; no displayed path is used as an existing native identity.

Executable and icon fields remember whether the user edited that path. Renaming
another field preserves the original native bytes or UTF-16, including paths that
cannot be represented as ordinary Unicode. A path edit creates a new native
literal only when it is unambiguous. Editing within a lossy parent while the
entered path still contains U+FFFD is refused; it never silently rewrites that
native parent. Keep the original path or enter a fully representable replacement.
Images are requested by the toolbar's
backend image service; this editor performs no filesystem or image loading.

Preview validates an immutable transient configuration and invokes the backend's
Preview command only. It neither applies settings nor executes processes. Once
accepted, Options closes and Jobs displays the captured preparation result. The
draft remains available when Options reopens. Disabled commands can be previewed;
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
The final settings target passes all 12 software-window cases on Windows and
Linux, with strict UI/application Clippy. The checks exercise pointer, keyboard
and widget accessibility actions; real-desktop acceptance is qualified separately.
