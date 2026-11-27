# Custom Commands settings

Options → Custom Commands edits the ordered configuration introduced in preferences schema 17.
New allocates a stable ID and immediately shows a selected **Unsaved draft** row.
That row is presentation only: Save validates and stages it as a configured
command within Options. The untouched incomplete draft shows no error, with Save
and Preview disabled. Changing the name, enabled state, label, icon or order
does not change its identity. Delete removes that command's shortcut overrides
through `UserPreferences::replace_user_commands`, preserving unrelated bindings.
The toolbar's Settings action selects the corresponding ID on this page.

Drag a list row to change command order; the insertion line shows its destination.
Dropping stages the new order within Options and keeps the dragged command
selected. OK persists it; Cancel restores the original order. Up/Down remain
available for keyboard and accessibility use. The list scrolls at its edges while dragging, and keyboard
navigation reveals the selected row. Dirty drafts require Save or
Discard before reordering. Escape, right-click, pointer cancellation, closing Options, stale
configuration and releases outside the actually visible list cancel the drag.

The gesture captures the command ID and immutable configuration revision, rather
than retaining a row index. Drop validates that capture before calling the
list transaction. Its visible bounds intersect the list and Options content scroll
viewport, including small-window and ancestor clipping, and exclude overlay
scrollbar controls. The interaction follows
Salamander's `src/edtlbwnd.cpp::MoveItem` and mouse/timer/cancel handling, with User
Menu's `EDTLBN_MOVEITEM2` integration in `src/dialogs4.cpp`; no reference source or
assets are copied.

The page follows Salamander's User Menu: Menu contents uses compact single-line
rows with +/−/↑/↓ header controls and room for six entries before scrolling.
The selected item's Command, Arguments and Initial directory fields appear below.
Row summaries remain available to accessibility and tooltips. The reference
layout is `src/lang/lang.rc:247`; field-specific menus and local editing are in `src/dialogs4.cpp:1980–2341` and
`src/execute.cpp:223/321/407/1965`. No reference assets or source are copied.

Program mode has an absolute native executable path and editable argument rows.
Each row is one argument, including empty rows; spaces and quotes are literal,
and no shell splitting is applied. Add/Remove and Up/Down edit those boundaries.
The argument value menu inserts captured file/folder values as complete arguments.
`${selected.paths}` is a whole Program argument in Single mode; the core rejects
invalid combinations. The Execution selector explicitly offers Run directly,
Run in terminal and Separator. Terminal mode edits a literal script with a shell;
it does not join the program's argument list into shell source. Switching modes
preserves uncommitted program and script fields. Captured-value declarations and
optional toolbar appearance are collapsible. An empty toolbar label remains an
icon-only button.

For Run directly the form shows **Output** below Execution (in narrow forms, below
it in the same column): **Hidden**, **Output window** or **Console window**, with
**Close when finished successfully** beside it for the output window. A new command
starts with Output window; commands saved before this choice existed stay Hidden.
Switching to Run in terminal or Separator keeps the choice in the draft but saves the
default for those modes. The choice is part of the staged command and is saved with
Options OK ([backend contract](user-commands.md#program-output)).

The output window (`command-output-window.slint`, `command_output_windows.rs`) is an
independent working window titled "<command> — output". Its first line says whether
the command runs (with an indeterminate progress bar), stops, finished, failed or was
stopped, with the exit code and duration; for a per-item command, how many runs ended
and how many failed. Below are the command line as run (display quotes only, in a
read-only field that can be selected and copied), the working folder and the start
time, then the output: standard error marked by an error-coloured bar and colour (and
"Error:" for screen readers), the window's own notices in italics. The list follows
the newest line until it is scrolled up and again once scrolled to the end; it scrolls
sideways to the longest line. **Stop** ends the program and every process it
started; **Copy all** copies the shown lines; **Save as…** asks for a file and, if
it exists, whether to replace it; **Clear** removes the lines shown so far; **Find**
(Ctrl+F, F3 and Shift+F3 or Next/Previous) finds lines containing the text,
ignoring case as Unicode lower case does (`str::to_lowercase`), and wraps around. A
search lower-cases each line into one reused buffer (ASCII lines byte by byte), so over
the 50,000 retained lines it allocates a fixed few times; only a line with a Greek
capital sigma, whose lower case depends on the letters around it, is lower-cased into
a string of its own
([record](../verification/2026-11-24-ui-perf-fixes.md#command-output-find)). **Close** and the window's close button stop a
program that still runs; Escape closes Find, or the window once nothing runs, and
never stops a program. **Close when finished successfully** is the command's own
setting, changed and saved from the window too; the window then closes by itself
when every run exited with code 0. Narrow windows show the buttons' glyphs only;
short ones leave out the details, the dropped-lines note and the check box.

A separator keeps its stable ID and position but has no execution fields or
preview. Converting a command to a separator removes its shortcut bindings;
the backend independently rejects forged Preview/Execute requests for it.

Save validates and stages one command, allowing another command to be
edited. The common OK footer also validates any remaining draft, saves the complete
Options transaction, and closes only after storage succeeds. A failed save keeps
the dialog open for retry. Discard changes reloads the currently staged command;
for a new unsaved command, it removes the presentation-only row. Cancel discards
the entire Options session, including staged commands and order.
Changing the selected command while a draft is dirty requires Save or
Discard. A configuration replacement invalidates stale
draft callbacks; no displayed path is used as an existing native identity.

The executable field remembers whether the user edited that path. Renaming
another field preserves the original native bytes or UTF-16, including paths that
cannot be represented as ordinary Unicode. An executable path edit creates a new
native literal only when it is unambiguous; edits within a lossy parent are refused.

Custom icon paths are read-only. Change icon and Browse image both import an
independent managed 128 × 128 PNG before returning a native result; Clear restores
the default icon. Form validation refuses a new typed path that lacks the captured
picker result, so it cannot bypass import. Existing unchanged legacy icon paths remain until artwork is reselected; there
is no background migration job. Every new choice uses managed storage.
The toolbar backend loads images;
this editor performs no filesystem or image loading.

Pictures are stored and removed as described in
[managed icon snapshots](#managed-icon-snapshots).

Browse buttons use the backend's native program, directory and image pickers.
Change icon uses the CommandIconResource native chooser; Browse image uses
CommandIconImage, leaving unrelated Image pickers unchanged. Both return the
managed PNG path to the same guarded icon draft.
Explicit artwork selections resolve symbolic links before the native provider
captures the regular target file, so linked Linux theme icons can be imported.
Format routing and import errors retain the selected filename. Broken links and
nonregular targets fail without publishing artwork.
Requests capture the target field, draft revision and configuration identity;
typing, changing pages or closing Options invalidates the result. Cancelling a
native Browse dialog leaves the draft unchanged and adds no error. An existing
validation or picker error remains visible; genuine picker failures are still
reported. Program picks retain the original native path, including
values whose displayed Unicode caption is lossy. Command image picks retain the imported native PNG path.
Valid completion revalidates the draft, enabling Save without an extra text edit.
Initial directory is
currently a template string: unrepresentable native directory picks are refused,
and dollar signs in a picked literal are escaped for that template. The Variables
menu beside Browse offers Active folder and Other folder. Choosing one replaces
the **entire** working-directory field with `${active.path}` or `${other.path}`;
it does not append to an existing directory. These templates retain the captured
native folder identity without converting it to a literal display path.

Required captured values shows the same complete shell reference that Insert
value inserts into the terminal script. Both use one formatter, and the six
labels follow the selected shell. Inserting a value also declares it as required;
toggling its required-value checkbox changes the declaration without inserting
script text.

| Captured value | PowerShell | POSIX shell | CMD |
| --- | --- | --- | --- |
| Active folder | `"$env:ME_ACTIVE_PATH"` | `"$ME_ACTIVE_PATH"` | `!ME_ACTIVE_PATH_ARG!` |
| Other folder | `"$env:ME_OTHER_PATH"` | `"$ME_OTHER_PATH"` | `!ME_OTHER_PATH_ARG!` |
| File path | `"$env:ME_FILE_PATH"` | `"$ME_FILE_PATH"` | `!ME_FILE_PATH_ARG!` |
| File name | `"$env:ME_FILE_NAME"` | `"$ME_FILE_NAME"` | `!ME_FILE_NAME_ARG!` |
| Name without extension | `"$env:ME_FILE_STEM"` | `"$ME_FILE_STEM"` | `!ME_FILE_STEM_ARG!` |
| Extension | `"$env:ME_FILE_EXTENSION"` | `"$ME_FILE_EXTENSION"` | `!ME_FILE_EXTENSION_ARG!` |

These are scalar references; `${selected.paths}` remains Program-only. CMD `_ARG`
references are complete backend-quoted external-program arguments and take no
additional quotes. Builtins need their own raw-value syntax, and CALL, batch files
and nested shells can reparse values; retain the
[backend shell limits](user-commands.md). Captured values remain data rather than
text interpolated into script syntax.

Options → Editors shares the argument-row control and a Browse button. Its draft
also survives page switches; Cancel discards it. Use the system default editor
or choose a program and include a complete File argument for F4. The + action
starts an incomplete custom-editor draft with visible executable/argument fields;
choosing the system default is explicit. A missing executable cannot be accepted.
OK validates the editor configuration once; editing or browsing never launches the editor.

Preview is a secondary inline dry run, requested explicitly rather than started
automatically. It validates an immutable transient configuration and invokes the
backend's Preview command only, with **zero process spawns**. It neither stages
nor saves settings. Once accepted, the originating command form displays the
captured preparation result inline while Options remains open with its draft.
It does not open the Jobs window. The
read-only result shows the program, working directory, separate argv elements and
per-item expansion, or the literal terminal script and captured environment. Its
display is bounded: at most four processes and sixteen arguments per program, each
value shortened to 256 bytes (`user_command_presentation.rs`), with argument and
environment values Debug-escaped;
omitted processes and arguments are stated. Jobs details show a captured preview
with the same bounded display.
Preview never runs the command or saves the draft. Disabled commands can be previewed;
execution remains disabled. Availability and native-shell errors are backend
results. See [the backend contract](user-commands.md) for provider and shell limits.

Keyboard settings uses `ShortcutMap::with_user_commands` for the catalog,
assignment checks, help and routing hints. Its immutable cache tracks both shortcut
overrides and command configuration identity. Rename/reorder preserves the selected
action by stable ID; deleting that action clears its pending candidate, so a late
Assign callback cannot bind a different action. Dynamic commands have no default
keys and are scoped to the file list outside filename search.

The page uses the Options scrolling body and fixed close footer. Its
validation debounce does not replace live text fields, preventing cursor resets
while editing. Command count, argument count and aggregate configuration limits
come from the core; malformed or oversized inputs are rejected with visible
validation instead of truncated.

Focused checks live in `presenter/user_command_settings_tests.rs`, the pure form
tests and `tests/user_command_settings_ui.rs`; they exercise pointer, keyboard and widget
accessibility actions in software windows. Evidence:
[settings record](../verification/2026-11-07-custom-command-settings.md),
[refactor record](../verification/2026-11-07-user-menu-refactor.md) and
[compact list record](../verification/2026-11-12-compact-custom-commands.md).

## Managed icon snapshots

Managed storage is the `icons` folder beside the preferences file. Each distinct
picture is one snapshot named by the SHA-256 of its PNG (`<64 hex digits>.png`), so
commands that chose the same picture share one file. The folder holds at most 1,024
snapshots; an import beyond that fails with "The managed command icon directory is
full." Publication and collection take the folder's `.snapshot.lock` without waiting,
so they never interleave, also across running instances.

Snapshots that nothing uses are removed after each successful preferences save
(`icon_snapshot::collect`, called by the Windows and Linux settings adapters). Under the
lock, a snapshot is removed only when all of these hold:

- its name is a snapshot name and it is a regular file: links, junctions, other
  reparse points, folders and every other entry stay, and so do link targets;
- the preferences just saved do not name it. Names come from every Custom Command's
  icon (by file name, wherever the path points) and from any snapshot name in a
  command kept for the other operating system;
- this process has not handed it to a picker. A pick that an open Options draft holds,
  or that was cancelled, stays until the application exits; a later session frees it;
- it was written, or last reused, more than seven days ago (`COLLECTION_GRACE`).
  Another running instance does not share this process's drafts or unsaved
  settings; the grace period keeps its recent picks. Reusing a snapshot older than a
  day restarts its period under the lock.

The rule prefers keeping a file to freeing space: removal errors keep the entry,
a busy lock skips the collection until a later save, and a save whose referenced
names and picks have not changed since the last collection does not list the folder
again. Known limits: a picture removed after a settings file was exported is not in
that file, which holds only the icon path, so a command imported from it later runs
without its picture; and an instance that keeps an older copy of the settings for
more than seven days after another instance added a picture can remove that picture
when it saves.
