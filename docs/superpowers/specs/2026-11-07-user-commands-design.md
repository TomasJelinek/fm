# Custom Commands

The Windows/Linux backend, settings, toolbar and shortcut integration are
implemented, including drag ordering. See [the settings verification](../../verification/2026-11-07-custom-command-settings.md)
and [the delivery ledger](../../IMPLEMENTATION_BACKLOG.md) for evidence and remaining capabilities.

## Purpose and interaction

Run a user-defined command in a chosen working directory with one click. A
dedicated toolbar sits immediately above the bottom command line. It occupies
no space when there are no configured commands. Each item has an icon and an
optional label to its right; a tooltip shows its name and launch summary.
Overflow uses horizontal scrolling with previous/next buttons, without making the
command line taller or increasing the application's 320×200 minimum.

Options gains a **Custom Commands** page with an ordered, selectable list,
Add/Edit/Remove, drag-and-drop ordering, move up/down and a preview. Drag a row
to an insertion marker; holding near a visible list edge scrolls the list. A
valid changed drop stages the new order and retains the command's ID and shortcut
bindings. Update command stages an edited form; OK saves the whole Options session
once, and Cancel restores its original list and appearance previews.
Escape or dropping outside the visible list cancels. Apply or cancel an edited
draft before changing order. The keyboard-accessible Up/Down buttons remain.
Right-click anywhere on this toolbar
opens a context menu containing **Settings…**, which opens that exact page and
selects the clicked command. An empty toolbar is configured from Options. A
settings selection, preview, icon hover or history recall never launches a command.

The Custom Commands page displays Preview results inline below its grouped draft
actions. Preview keeps Options open and does not create another Jobs popup. The
result shows the working folder, program and separate arguments (or literal
terminal script and captured environment) in a bounded, scrollable read-only
area. It belongs to one draft and preview attempt: editing, selecting another
command or cancelling invalidates that display. A delayed failure from an older
attempt must not overwrite a newer result. Selection capture and validation
errors stay in the same form. Update command stages the draft; only the main
Options OK saves it, and Cancel discards it.

## Salamander reference and adaptation

Reviewed `D:/experiments/salamander-main/src/usermenu.h` (CUserMenuItem),
`dialogs4.cpp` (CCfgPageUserMenu), `mainwnd4.cpp` (ExpandCommand2) and
`execute.cpp` (ValidateUserMenuArguments/ExpandUserMenuArguments). Salamander
separates name, executable, arguments, initial directory and icon, supports
shell/terminal/window choices, toolbar visibility, groups/separators and
selection-dependent expansion. Its validation prevents mixing grouped and
per-item expansion. Its fixed ANSI buffers and short-path conversion are not
appropriate here: retain native Unicode paths and explicit platform boundaries.
This is behavioral inspiration; no source code is copied.

## Model and execution

`UserCommand` has a stable ID, name, enabled flag, optional toolbar text, icon
reference, working-directory template, invocation mode and arguments. Keep a
bounded list (128 commands, 64 stored argument templates per direct launch); these are resource
budgets, not global path length limits. Settings are versioned and written by the
existing backend settings service. Duplicate IDs are rejected; duplicate labels
are allowed. Shortcut actions use `user-command.<id>` in the configurable map.

Two explicit modes avoid ambiguous quoting:

- **Program:** native executable path plus an argument list. Pass each expanded
  argument separately to the platform process API; never join it into a shell
  command. Use this as the default. Executable choices are explicit, independent
  of the browsed directory's PATH or file associations.
- **Terminal command:** choose an available shell/terminal profile and enter its
  script text. Run as ordinary user, with the same terminal service as the command
  bar. Script syntax is deliberately user-authored. A directory path is passed
  as the working directory, never concatenated into `cd ... && ...`.

Templates support `${active.path}`, `${other.path}`, `${file.path}`,
`${file.name}`, `${file.stem}`, `${file.extension}` and `${selected.paths}`.
The last expands only as whole argument-list elements in Program mode. A
per-item launch option explicitly executes once per captured selected item;
group and per-item expansion cannot be combined. Shell mode initially supports
scalar variables only through backend-created process environment values, with
shell-specific reference snippets inserted by the picker. Never interpolate
untrusted file names directly into script syntax. Environment values are data,
not subject to recursive template expansion. Missing variables and embedded NUL
fail before execution. Preview shows resolved values with native identities
retained separately; it is never an executable command reconstruction.

The initial runtime accepts at most 4,096 captured items and 4,160 expanded native
arguments per process. The stored command list has a 1 MiB aggregate budget.
Preparation/dispatch are serialized; at most eight launched child processes are
owned concurrently across custom commands, terminal and editor operations. A
retained preview keeps its backend memory reservation until the UI releases it.
These bounds limit one operation's resources, not valid filesystem path lengths.

Windows uses Program, PowerShell or explicitly qualified Cmd; Linux uses Program
or its POSIX terminal profile. Cmd keeps one literal command line and exact raw
`ME_*` values, and provides backend-quoted `ME_*_ARG` companions for direct native
argv references such as `!ME_FILE_PATH_ARG!`. Builtins and reparsing commands have
different semantics; the precise limits and native evidence are in the
[backend contract](../../developer/USER_COMMANDS.md). Ordinary command-bar Cmd is
unchanged. Shell variables remain explicit declarations from the fixed `ME_*` set;
undeclared inherited raw and companion values are removed in the child.

Capture panel/selection/working directory at click time, resolve and validate on
a bounded backend worker, then launch through separate Windows/Linux/macOS
implementations. Archive member placeholders require an explicit future export
policy and initially give an unsupported-source message; displayed archive paths
are not executable native paths. Do not silently elevate or save credentials.
Use the existing job/status error presentation for failures. Never log full
command text or persist it into command history without the user's action.

Jobs distinguishes preparation, process starts and dispatch completion. Completion
means the requested programs were launched, not that they exited successfully.
Cancel prevents future launches and does not kill already started applications.
Preview launches nothing and is itself terminal; it does not await a later finish
event. Only the latest preview retains expanded details, which are released on a
backend reclamation worker when another activation supersedes it. The display
shows bounded native values with explicit truncation, never executable joined argv.

## Icons, accessibility and testing

Prefer an explicitly chosen local icon, otherwise the executable's system icon,
otherwise the app's generic command icon. Resolve and decode icons on backend
workers with the established image limits/cache. Store a path/reference rather
than unbounded embedded data. Theme and density determine presentation; keyboard
focus, accessible labels and disabled states are required.

The current implementation loads explicit image icons through the isolated image
service only while visible, and otherwise uses the generic Run icon. Executable
artwork remains a separate native capability to implement; the existing file-type
icon cache does not inspect arbitrary executable contents.

Implementation tests must cover Unicode/long native paths, quote/semicolon/dollar
characters, raw non-Unicode Unix names, missing working directories, stale
selection, empty selections, argument expansion cardinality, cancellation before
launch, malformed settings/icons, right-click settings routing, hidden empty
toolbar and compact overflow. Each platform tests the actual argv and working
directory received by a helper process; it must not test a simulated shell string.
